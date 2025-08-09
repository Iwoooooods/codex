use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;
use tracing::debug;
use tracing::info;
use tracing::warn;

use tree_sitter::Node;
use tree_sitter::Parser;

use crate::walk_utils::is_supported_file_extension;
use crate::walk_utils::walk_codebase_files;
use tree_sitter::Tree;

use crate::file_state::CodebaseState;
use crate::file_state::FileState;

/// Represents a code symbol that can be indexed for semantic search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    /// The name of the symbol (function name, struct name, etc.)
    pub name: String,
    /// The type of symbol (function, struct, class, etc.)
    pub kind: SymbolKind,
    /// The full source code text of this symbol
    pub content: String,
    /// File path where this symbol is located
    pub file_path: PathBuf,
    /// Start line number (1-indexed)
    pub start_line: usize,
    /// End line number (1-indexed)
    pub end_line: usize,
    /// Start column (0-indexed)
    pub start_column: usize,
    /// End column (0-indexed)
    pub end_column: usize,
    /// Additional context (e.g., class name for methods)
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    Constant,
    Variable,
    Class,
    Method,
    Interface,
    Type,
}

/// Supported programming languages for parsing
#[derive(Debug, Clone)]
pub enum SupportedLanguage {
    // Add more languages as needed
    Rust,
    Python,
    Go,
}

impl SupportedLanguage {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "rs" => Some(SupportedLanguage::Rust),
            "py" => Some(SupportedLanguage::Python),
            "go" => Some(SupportedLanguage::Go),
            _ => None,
        }
    }

    pub fn tree_sitter_language(&self) -> tree_sitter::Language {
        match self {
            SupportedLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
            SupportedLanguage::Python => tree_sitter_python::LANGUAGE.into(),
            SupportedLanguage::Go => tree_sitter_go::LANGUAGE.into(),
        }
    }

    /// Get the file extensions supported by this language
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            SupportedLanguage::Rust => &["rs"],
            SupportedLanguage::Python => &["py"],
            SupportedLanguage::Go => &["go"],
        }
    }
}

/// Parser for extracting symbols from source code using tree-sitter
pub struct SymbolParser {
    pub parsers: HashMap<String, Parser>,
}

impl SymbolParser {
    pub fn new() -> Result<Self, anyhow::Error> {
        let mut parsers = HashMap::new();

        // Initialize Rust parser
        let mut rust_parser = Parser::new();
        match rust_parser.set_language(&SupportedLanguage::Rust.tree_sitter_language()) {
            Ok(_) => (),
            Err(e) => return Err(anyhow::anyhow!("Failed to set language: {e}")),
        };
        parsers.insert("rs".to_string(), rust_parser);

        // Initialize Python parser
        let mut python_parser = Parser::new();
        match python_parser.set_language(&SupportedLanguage::Python.tree_sitter_language()) {
            Ok(_) => (),
            Err(e) => return Err(anyhow::anyhow!("Failed to set Python language: {e}")),
        };
        parsers.insert("py".to_string(), python_parser);

        // Initialize Go parser
        let mut go_parser = Parser::new();
        match go_parser.set_language(&SupportedLanguage::Go.tree_sitter_language()) {
            Ok(_) => (),
            Err(e) => return Err(anyhow::anyhow!("Failed to set Go language: {e}")),
        };
        parsers.insert("go".to_string(), go_parser);

        Ok(SymbolParser { parsers })
    }

    /// Parse a single file and extract all symbols
    pub fn parse_file<P: AsRef<Path>>(
        &mut self,
        file_path: P,
    ) -> Result<Vec<Symbol>, anyhow::Error> {
        let content = fs::read_to_string(file_path.as_ref())?;
        let extension = file_path
            .as_ref()
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        let language = SupportedLanguage::from_extension(extension)
            .ok_or_else(|| anyhow::anyhow!("Unsupported file extension: {extension}"))?;

        let parser = self
            .parsers
            .get_mut(extension)
            .ok_or_else(|| anyhow::anyhow!("No parser available for extension: {extension}"))?;

        let tree = parser
            .parse(&content, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse file"))?;

        let symbols = self.extract_symbols(&tree, &content, file_path.as_ref(), &language)?;

        debug!(
            "Extracted {} symbols from {}",
            symbols.len(),
            file_path.as_ref().display()
        );
        Ok(symbols)
    }

    /// Extract symbols from a parsed tree
    pub fn extract_symbols(
        &self,
        tree: &Tree,
        source: &str,
        file_path: &Path,
        language: &SupportedLanguage,
    ) -> Result<Vec<Symbol>, anyhow::Error> {
        let mut symbols = Vec::new();
        let root_node = tree.root_node();

        match language {
            SupportedLanguage::Rust => {
                self.extract_rust_symbols(root_node, source, file_path, &mut symbols)?;
            }
            SupportedLanguage::Python => {
                self.extract_python_symbols(root_node, source, file_path, &mut symbols)?;
            }
            SupportedLanguage::Go => {
                self.extract_go_symbols(root_node, source, file_path, &mut symbols)?;
            }
        }

        Ok(symbols)
    }

    /// Extract symbols from Rust code
    fn extract_rust_symbols(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
    ) -> Result<(), anyhow::Error> {
        self.traverse_rust_node(node, source, file_path, symbols, None)?;
        Ok(())
    }

    fn extract_python_symbols(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
    ) -> Result<(), anyhow::Error> {
        self.traverse_python_node(node, source, file_path, symbols, None)?;
        Ok(())
    }

    fn extract_go_symbols(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
    ) -> Result<(), anyhow::Error> {
        self.traverse_go_node(node, source, file_path, symbols, None)?;
        Ok(())
    }

    /// Recursively traverse Rust AST nodes to find symbols
    fn traverse_rust_node(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
        context: Option<String>,
    ) -> Result<(), anyhow::Error> {
        match node.kind() {
            "function_item" => {
                if let Some(symbol) =
                    self.extract_rust_function(node, source, file_path, &context)?
                {
                    symbols.push(symbol);
                }
            }
            "struct_item" => {
                if let Some(symbol) = self.extract_rust_struct(node, source, file_path, &context)? {
                    let struct_name = symbol.name.clone();
                    symbols.push(symbol);

                    // For struct implementations, pass the struct name as context
                    for child in node.children(&mut node.walk()) {
                        self.traverse_rust_node(
                            child,
                            source,
                            file_path,
                            symbols,
                            Some(struct_name.clone()),
                        )?;
                    }
                    return Ok(());
                }
            }
            "enum_item" => {
                if let Some(symbol) = self.extract_rust_enum(node, source, file_path, &context)? {
                    symbols.push(symbol);
                }
            }
            "trait_item" => {
                if let Some(symbol) = self.extract_rust_trait(node, source, file_path, &context)? {
                    symbols.push(symbol);
                }
            }
            "impl_item" => {
                if let Some(symbol) = self.extract_rust_impl(node, source, file_path, &context)? {
                    let impl_context = Some(symbol.name.clone());
                    symbols.push(symbol);

                    // Extract methods from impl block
                    for child in node.children(&mut node.walk()) {
                        self.traverse_rust_node(
                            child,
                            source,
                            file_path,
                            symbols,
                            impl_context.clone(),
                        )?;
                    }
                    return Ok(());
                }
            }
            "const_item" | "static_item" => {
                if let Some(symbol) =
                    self.extract_rust_constant(node, source, file_path, &context)?
                {
                    symbols.push(symbol);
                }
            }
            "mod_item" => {
                if let Some(symbol) = self.extract_rust_module(node, source, file_path, &context)? {
                    symbols.push(symbol);
                }
            }
            _ => {}
        }

        // Continue traversing child nodes
        for child in node.children(&mut node.walk()) {
            self.traverse_rust_node(child, source, file_path, symbols, context.clone())?;
        }

        Ok(())
    }

    /// Extract function symbol from Rust code
    fn extract_rust_function(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        // Find function name
        let name = self
            .find_child_text(node, "identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Function missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        let kind = if context.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };

        Ok(Some(Symbol {
            name,
            kind,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract struct symbol from Rust code
    fn extract_rust_struct(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        let name = self
            .find_child_text(node, "type_identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Struct missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        Ok(Some(Symbol {
            name,
            kind: SymbolKind::Struct,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract enum symbol from Rust code
    fn extract_rust_enum(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        let name = self
            .find_child_text(node, "type_identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Enum missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        Ok(Some(Symbol {
            name,
            kind: SymbolKind::Enum,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract trait symbol from Rust code
    fn extract_rust_trait(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        let name = self
            .find_child_text(node, "type_identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Trait missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        Ok(Some(Symbol {
            name,
            kind: SymbolKind::Trait,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract impl symbol from Rust code
    fn extract_rust_impl(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        // Find the type being implemented
        let name = self
            .find_child_text(node, "type_identifier", source)?
            .unwrap_or_else(|| "impl".to_string());

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        Ok(Some(Symbol {
            name: format!("impl {name}"),
            kind: SymbolKind::Impl,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract constant symbol from Rust code
    fn extract_rust_constant(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        let name = self
            .find_child_text(node, "identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Constant missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        Ok(Some(Symbol {
            name,
            kind: SymbolKind::Constant,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract module symbol from Rust code
    fn extract_rust_module(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        let name = self
            .find_child_text(node, "identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Module missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        Ok(Some(Symbol {
            name,
            kind: SymbolKind::Module,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Recursively traverse Python AST nodes to find symbols
    fn traverse_python_node(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
        context: Option<String>,
    ) -> Result<(), anyhow::Error> {
        match node.kind() {
            "function_definition" => {
                if let Some(symbol) =
                    self.extract_python_function(node, source, file_path, &context)?
                {
                    symbols.push(symbol);
                }
            }
            "class_definition" => {
                if let Some(symbol) =
                    self.extract_python_class(node, source, file_path, &context)?
                {
                    let class_name = symbol.name.clone();
                    symbols.push(symbol);

                    // For class methods, pass the class name as context
                    for child in node.children(&mut node.walk()) {
                        self.traverse_python_node(
                            child,
                            source,
                            file_path,
                            symbols,
                            Some(class_name.clone()),
                        )?;
                    }
                    return Ok(());
                }
            }
            _ => {}
        }

        // Continue traversing child nodes
        for child in node.children(&mut node.walk()) {
            self.traverse_python_node(child, source, file_path, symbols, context.clone())?;
        }

        Ok(())
    }

    /// Extract function symbol from Python code
    fn extract_python_function(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        // Find function name
        let name = self
            .find_child_text(node, "identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Python function missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        let kind = if context.is_some() {
            SymbolKind::Method
        } else {
            SymbolKind::Function
        };

        Ok(Some(Symbol {
            name,
            kind,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract class symbol from Python code
    fn extract_python_class(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        let name = self
            .find_child_text(node, "identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Python class missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        Ok(Some(Symbol {
            name,
            kind: SymbolKind::Class,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Recursively traverse Go AST nodes to find symbols
    fn traverse_go_node(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        symbols: &mut Vec<Symbol>,
        context: Option<String>,
    ) -> Result<(), anyhow::Error> {
        match node.kind() {
            "function_declaration" => {
                if let Some(symbol) = self.extract_go_function(node, source, file_path, &context)? {
                    symbols.push(symbol);
                }
            }
            "method_declaration" => {
                if let Some(symbol) = self.extract_go_method(node, source, file_path, &context)? {
                    symbols.push(symbol);
                }
            }
            "type_declaration" => {
                // Go type declarations can contain structs, interfaces, etc.
                for child in node.children(&mut node.walk()) {
                    if child.kind() == "type_spec" {
                        if let Some(symbol) =
                            self.extract_go_type(child, source, file_path, &context)?
                        {
                            symbols.push(symbol);
                        }
                    }
                }
            }
            "const_declaration" | "var_declaration" => {
                if let Some(symbol) = self.extract_go_variable(node, source, file_path, &context)? {
                    symbols.push(symbol);
                }
            }
            _ => {}
        }

        // Continue traversing child nodes
        for child in node.children(&mut node.walk()) {
            self.traverse_go_node(child, source, file_path, symbols, context.clone())?;
        }

        Ok(())
    }

    /// Extract function symbol from Go code
    fn extract_go_function(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        // Find function name
        let name = self
            .find_child_text(node, "identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Go function missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        Ok(Some(Symbol {
            name,
            kind: SymbolKind::Function,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract method symbol from Go code
    fn extract_go_method(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        // Find method name
        let name = self
            .find_child_text(node, "field_identifier", source)?
            .or_else(|| {
                self.find_child_text(node, "identifier", source)
                    .unwrap_or(None)
            })
            .ok_or_else(|| anyhow::anyhow!("Go method missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        // Try to extract receiver type for context
        let receiver_context = self.extract_go_receiver_type(node, source)?;
        let final_context = receiver_context.or_else(|| context.clone());

        Ok(Some(Symbol {
            name,
            kind: SymbolKind::Method,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: final_context,
        }))
    }

    /// Extract type symbol from Go code (struct, interface, etc.)
    fn extract_go_type(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        // Find type name
        let name = self
            .find_child_text(node, "type_identifier", source)?
            .ok_or_else(|| anyhow::anyhow!("Go type missing name"))?;

        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        // Determine the kind based on the type
        let kind = if content.contains("struct") {
            SymbolKind::Struct
        } else if content.contains("interface") {
            SymbolKind::Interface
        } else {
            SymbolKind::Type
        };

        Ok(Some(Symbol {
            name,
            kind,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract variable/constant symbol from Go code
    fn extract_go_variable(
        &self,
        node: Node,
        source: &str,
        file_path: &Path,
        context: &Option<String>,
    ) -> Result<Option<Symbol>, anyhow::Error> {
        // Find variable name - could be in a var_spec or const_spec child
        let mut name = None;
        for child in node.children(&mut node.walk()) {
            if child.kind() == "var_spec" || child.kind() == "const_spec" {
                name = self.find_child_text(child, "identifier", source)?;
                if name.is_some() {
                    break;
                }
            }
        }

        let name = name.ok_or(anyhow::anyhow!("Go variable/constant missing name"))?;
        let content = node.utf8_text(source.as_bytes())?;
        let start_pos = node.start_position();
        let end_pos = node.end_position();

        let kind = if node.kind() == "const_declaration" {
            SymbolKind::Constant
        } else {
            SymbolKind::Variable
        };

        Ok(Some(Symbol {
            name,
            kind,
            content: content.to_string(),
            file_path: file_path.to_path_buf(),
            start_line: start_pos.row + 1,
            end_line: end_pos.row + 1,
            start_column: start_pos.column,
            end_column: end_pos.column,
            context: context.clone(),
        }))
    }

    /// Extract receiver type from Go method declaration
    fn extract_go_receiver_type(
        &self,
        node: Node,
        source: &str,
    ) -> Result<Option<String>, anyhow::Error> {
        for child in node.children(&mut node.walk()) {
            if child.kind() == "parameter_list" {
                // This is likely the receiver
                if let Some(receiver_type) =
                    self.find_child_text(child, "type_identifier", source)?
                {
                    return Ok(Some(receiver_type));
                }
            }
        }
        Ok(None)
    }

    /// Helper function to find text content of a child node with specific kind
    fn find_child_text(
        &self,
        node: Node,
        kind: &str,
        source: &str,
    ) -> Result<Option<String>, anyhow::Error> {
        for child in node.children(&mut node.walk()) {
            if child.kind() == kind {
                let text = child.utf8_text(source.as_bytes())?;
                return Ok(Some(text.to_string()));
            }
        }
        Ok(None)
    }
}

/// Helper function to extract file metadata (last modified time)
pub fn get_file_metadata(path: &Path) -> Result<u64, anyhow::Error> {
    let metadata = fs::metadata(path)
        .map_err(|e| anyhow::anyhow!("Failed to get metadata for '{}': {}", path.display(), e))?;

    let last_modified = metadata
        .modified()
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to get modified time for '{}': {}",
                path.display(),
                e
            )
        })?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| {
            anyhow::anyhow!(
                "Failed to get last modified time for {}: {}",
                path.display(),
                e
            )
        })?
        .as_secs();

    Ok(last_modified)
}

/// Index a codebase by walking through directories and extracting symbols
pub fn parse_codebase<P: AsRef<Path>>(root_path: P) -> Result<Vec<Symbol>, anyhow::Error> {
    let mut parser = SymbolParser::new()?;
    let mut all_symbols = Vec::new();
    let mut file_state_map = HashMap::new();

    info!(
        "Starting codebase indexing at: {}",
        root_path.as_ref().display()
    );

    walk_codebase_files(root_path.as_ref(), |path| {
        // Only process supported file types
        if !is_supported_file_extension(path) {
            return Ok(true); // Continue walking
        }

        // Get file metadata
        let last_modified = match get_file_metadata(path) {
            Ok(timestamp) => timestamp,
            Err(e) => {
                warn!("Skipping file due to metadata error: {}", e);
                return Ok(true); // Continue walking
            }
        };

        let file_state = FileState::new(path.to_string_lossy().to_string(), last_modified)
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to create file state for '{}': {}",
                    path.display(),
                    e
                )
            })?;
        file_state_map.insert(path.to_string_lossy().to_string(), file_state);

        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if SupportedLanguage::from_extension(extension).is_some() {
            debug!("Processing file: {}", path.display());

            match parser.parse_file(path) {
                Ok(mut symbols) => {
                    info!(
                        "Extracted {} symbols from {}",
                        symbols.len(),
                        path.display()
                    );
                    all_symbols.append(&mut symbols);
                }
                Err(e) => {
                    warn!("Failed to parse '{}': {}", path.display(), e);
                }
            }
        }
        Ok(true) // Continue walking
    })?;

    let codebase_state = CodebaseState {
        file_states: file_state_map,
    };
    codebase_state
        .to_file(None)
        .map_err(|e| anyhow::anyhow!("Failed to save codebase state to index.json: {}", e))?;

    info!(
        "Indexing complete. Total symbols extracted: {}",
        all_symbols.len()
    );
    Ok(all_symbols)
}
