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

use tree_sitter::Tree;
use walkdir::WalkDir;

use crate::file_state::CodebaseState;
use crate::file_state::FileState;
use crate::test_data;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    Rust,
    // Add more languages as needed
    // JavaScript,
    // Python,
    // TypeScript,
}

impl SupportedLanguage {
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "rs" => Some(SupportedLanguage::Rust),
            _ => None,
        }
    }

    pub fn tree_sitter_language(&self) -> tree_sitter::Language {
        match self {
            SupportedLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
        }
    }

    /// Get the file extensions supported by this language
    pub fn extensions(&self) -> &'static [&'static str] {
        match self {
            SupportedLanguage::Rust => &["rs"],
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

        Ok(SymbolParser { parsers })
    }

    /// Parse a single file and extract all symbols
    pub fn parse_file(
        &mut self,
        file_path: &Path,
    ) -> Result<Vec<Symbol>, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(file_path)?;
        let extension = match file_path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => ext,
            None => "",
        };

        let language = SupportedLanguage::from_extension(extension)
            .ok_or_else(|| format!("Unsupported file extension: {extension}"))?;

        let parser = self
            .parsers
            .get_mut(extension)
            .ok_or_else(|| format!("No parser available for extension: {extension}"))?;

        let tree = parser.parse(&content, None).ok_or("Failed to parse file")?;

        let symbols = self.extract_symbols(&tree, &content, file_path, &language)?;

        debug!(
            "Extracted {} symbols from {}",
            symbols.len(),
            file_path.display()
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
    ) -> Result<Vec<Symbol>, Box<dyn std::error::Error>> {
        let mut symbols = Vec::new();
        let root_node = tree.root_node();

        match language {
            SupportedLanguage::Rust => {
                self.extract_rust_symbols(root_node, source, file_path, &mut symbols)?;
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
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.traverse_rust_node(node, source, file_path, symbols, None)?;
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
    ) -> Result<(), Box<dyn std::error::Error>> {
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
    ) -> Result<Option<Symbol>, Box<dyn std::error::Error>> {
        // Find function name
        let name = self
            .find_child_text(node, "identifier", source)?
            .ok_or("Function missing name")?;

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
    ) -> Result<Option<Symbol>, Box<dyn std::error::Error>> {
        let name = self
            .find_child_text(node, "type_identifier", source)?
            .ok_or("Struct missing name")?;

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
    ) -> Result<Option<Symbol>, Box<dyn std::error::Error>> {
        let name = self
            .find_child_text(node, "type_identifier", source)?
            .ok_or("Enum missing name")?;

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
    ) -> Result<Option<Symbol>, Box<dyn std::error::Error>> {
        let name = self
            .find_child_text(node, "type_identifier", source)?
            .ok_or("Trait missing name")?;

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
    ) -> Result<Option<Symbol>, Box<dyn std::error::Error>> {
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
    ) -> Result<Option<Symbol>, Box<dyn std::error::Error>> {
        let name = self
            .find_child_text(node, "identifier", source)?
            .ok_or("Constant missing name")?;

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
    ) -> Result<Option<Symbol>, Box<dyn std::error::Error>> {
        let name = self
            .find_child_text(node, "identifier", source)?
            .ok_or("Module missing name")?;

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

    /// Helper function to find text content of a child node with specific kind
    fn find_child_text(
        &self,
        node: Node,
        kind: &str,
        source: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        for child in node.children(&mut node.walk()) {
            if child.kind() == kind {
                let text = child.utf8_text(source.as_bytes())?;
                return Ok(Some(text.to_string()));
            }
        }
        Ok(None)
    }
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

    for entry in WalkDir::new(root_path).follow_links(false) {
        let entry = entry.map_err(|e| anyhow::anyhow!("Failed to walk directory: {}", e))?;
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        // generate file state only for files
        let last_modified = match entry
            .metadata()
            .map_err(|e| anyhow::anyhow!("Failed to get metadata for '{}': {}", path.display(), e))?
            .modified()
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to get modified time for '{}': {}",
                    path.display(),
                    e
                )
            })?
            .duration_since(UNIX_EPOCH)
        {
            Ok(duration) => duration.as_secs(),
            Err(e) => {
                warn!(
                    "Failed to get last modified time for {}: {}",
                    path.display(),
                    e
                );
                continue;
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

        let extension = match path.extension().and_then(|ext| ext.to_str()) {
            Some(ext) => ext,
            None => "",
        };

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
    }

    let codebase_state = CodebaseState {
        file_states: file_state_map,
    };
    codebase_state
        .to_file()
        .map_err(|e| anyhow::anyhow!("Failed to save codebase state to index.json: {}", e))?;

    info!(
        "Indexing complete. Total symbols extracted: {}",
        all_symbols.len()
    );
    Ok(all_symbols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Node;

    fn print_node_info(node: Node, depth: usize) {
        let indent = "  ".repeat(depth);
        debug!(
            "{}Node: {:?} at {}:{}",
            indent,
            node.kind(),
            node.start_position().row,
            node.start_position().column
        );

        if node.child_count() > 0 {
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    print_node_info(child, depth + 1);
                }
            }
        }
    }

    fn find_functions(node: Node) -> Vec<String> {
        let mut functions = Vec::new();

        if node.kind() == "function_item" {
            // Find the function name
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "identifier" {
                        let text =
                            match child.utf8_text(test_data::TEST_RUST_CODE_SIMPLE.as_bytes()) {
                                Ok(text) => text,
                                Err(e) => panic!("Error getting text: {e}"),
                            };
                        functions.push(text.to_string());
                        break;
                    }
                }
            }
        }

        // Recursively search children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                functions.extend(find_functions(child));
            }
        }

        functions
    }

    fn find_structs(node: Node) -> Vec<String> {
        let mut structs = Vec::new();

        if node.kind() == "struct_item" {
            // Find the struct name
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "type_identifier" {
                        let text =
                            match child.utf8_text(test_data::TEST_RUST_CODE_SIMPLE.as_bytes()) {
                                Ok(text) => text,
                                Err(e) => panic!("Error getting text: {e}"),
                            };
                        structs.push(text.to_string());
                        break;
                    }
                }
            }
        }

        // Recursively search children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                structs.extend(find_structs(child));
            }
        }

        structs
    }

    #[test]
    fn symbol_analysis() {
        let _ = tracing_subscriber::fmt::try_init();

        let mut parser = Parser::new();
        match parser.set_language(&tree_sitter_rust::LANGUAGE.into()) {
            Ok(parser) => parser,
            Err(e) => panic!("Error loading Rust grammar: {e}"),
        };

        let tree = match parser.parse(test_data::TEST_RUST_CODE_SIMPLE, None) {
            Some(tree) => tree,
            None => panic!("Error parsing source code"),
        };
        let root_node = tree.root_node();

        debug!("=== Tree Structure Analysis ===");
        print_node_info(root_node, 0);

        debug!("=== Function Analysis ===");
        let functions = find_functions(root_node);
        debug!("Found functions: {:?}", functions);

        debug!("=== Struct Analysis ===");
        let structs = find_structs(root_node);
        debug!("Found structs: {:?}", structs);

        // Assertions about the structure
        assert!(functions.contains(&"new".to_string()));
        assert!(functions.contains(&"add_hobby".to_string()));
        assert!(functions.contains(&"get_info".to_string()));
        assert!(functions.contains(&"create_people".to_string()));
        assert!(functions.contains(&"test_person_creation".to_string()));

        assert!(structs.contains(&"Person".to_string()));

        // Check that we found the expected number of functions
        assert_eq!(functions.len(), 5);
        assert_eq!(structs.len(), 1);

        debug!("=== Test completed successfully ===");
    }

    #[test]
    fn test_symbol_extraction() {
        let _ = tracing_subscriber::fmt::try_init();

        // Create a temporary file with our test Rust code
        let temp_dir = std::env::temp_dir();
        let test_file = temp_dir.join("test_symbols.rs");
        match std::fs::write(&test_file, test_data::TEST_RUST_CODE_SIMPLE) {
            Ok(_) => (),
            Err(e) => panic!("Failed to write test file: {e}"),
        };

        // Create symbol parser and parse the file
        let mut parser = match SymbolParser::new() {
            Ok(parser) => parser,
            Err(e) => panic!("Failed to create parser: {e}"),
        };
        let symbols = match parser.parse_file(&test_file) {
            Ok(symbols) => symbols,
            Err(e) => panic!("Failed to parse file: {e}"),
        };

        // Clean up the temporary file
        std::fs::remove_file(&test_file).ok();

        debug!("=== Symbol Extraction Results ===");
        for symbol in &symbols {
            debug!(
                "Symbol: {} ({:?}) at {}:{}-{}:{} in {}",
                symbol.name,
                symbol.kind,
                symbol.start_line,
                symbol.start_column,
                symbol.end_line,
                symbol.end_column,
                symbol.file_path.display()
            );
            if let Some(ref context) = symbol.context {
                debug!("  Context: {}", context);
            }
        }

        // Verify we found the expected symbols
        let function_symbols: Vec<_> = symbols
            .iter()
            .filter(|s| matches!(s.kind, SymbolKind::Function | SymbolKind::Method))
            .map(|s| s.name.as_str())
            .collect();

        let struct_symbols: Vec<_> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .map(|s| s.name.as_str())
            .collect();

        // Check functions and methods
        assert!(function_symbols.contains(&"new"));
        assert!(function_symbols.contains(&"add_hobby"));
        assert!(function_symbols.contains(&"get_info"));
        assert!(function_symbols.contains(&"create_people"));
        assert!(function_symbols.contains(&"test_person_creation"));

        // Check structs
        assert!(struct_symbols.contains(&"Person"));

        // Verify we have the expected counts (might be more due to impl blocks, etc.)
        assert!(function_symbols.len() >= 5);
        assert!(!struct_symbols.is_empty());

        // Check that all symbols have valid content
        for symbol in &symbols {
            assert!(
                !symbol.content.is_empty(),
                "Symbol '{}' has empty content",
                symbol.name
            );
            assert!(
                symbol.start_line > 0,
                "Symbol '{}' has invalid start line",
                symbol.name
            );
            assert!(
                symbol.end_line >= symbol.start_line,
                "Symbol '{}' has invalid line range",
                symbol.name
            );
        }

        info!(
            "Symbol extraction test completed successfully. Found {} symbols",
            symbols.len()
        );
    }

    #[test]
    fn test_codebase_indexing() {
        let _ = tracing_subscriber::fmt::try_init();

        // Create a temporary directory structure with test files
        let temp_dir = std::env::temp_dir().join("test_codebase");
        let src_dir = temp_dir.join("src");
        match std::fs::create_dir_all(&src_dir) {
            Ok(_) => (),
            Err(e) => panic!("Failed to create test directory: {e}"),
        };

        // Create test files
        let lib_file = src_dir.join("lib.rs");
        let main_file = src_dir.join("main.rs");

        match std::fs::write(&lib_file, test_data::TEST_RUST_CODE_SIMPLE) {
            Ok(_) => (),
            Err(e) => panic!("Failed to write lib.rs: {e}"),
        };
        match std::fs::write(&main_file, "fn main() { println!(\"Hello, world!\"); }") {
            Ok(_) => (),
            Err(e) => panic!("Failed to write main.rs: {e}"),
        };

        // Index the codebase
        let symbols = match parse_codebase(&temp_dir) {
            Ok(symbols) => symbols,
            Err(e) => panic!(
                "Failed to index codebase at '{}': {}",
                temp_dir.display(),
                e
            ),
        };

        // Before we clean up, we should check if the codebase state is created
        let codebase_state = match CodebaseState::from_file() {
            Ok(codebase_state) => codebase_state,
            Err(e) => panic!("Failed to read codebase state from index.json: {}", e),
        };
        assert_eq!(codebase_state.file_states.len(), 2);
        let lib_file_path = lib_file.to_string_lossy().to_string();
        let main_file_path = main_file.to_string_lossy().to_string();
        let lib_file_state = match codebase_state.file_states.get(&lib_file_path) {
            Some(file_state) => file_state,
            None => panic!("Failed to get lib.rs file state"),
        };
        let main_file_state = match codebase_state.file_states.get(&main_file_path) {
            Some(file_state) => file_state,
            None => panic!("Failed to get main.rs file state"),
        };
        assert!(
            lib_file_state.last_modified > 0,
            "lib.rs should have a valid timestamp"
        );
        assert!(
            main_file_state.last_modified > 0,
            "main.rs should have a valid timestamp"
        );

        // Clean up
        std::fs::remove_dir_all(&temp_dir).ok();

        debug!("=== Codebase Indexing Results ===");
        debug!("Total symbols found: {}", symbols.len());

        // Checks
        // 1. Group symbols by file
        let mut symbols_by_file: HashMap<PathBuf, Vec<&Symbol>> = HashMap::new();
        for symbol in &symbols {
            symbols_by_file
                .entry(symbol.file_path.clone())
                .or_default()
                .push(symbol);
        }

        for (file_path, file_symbols) in &symbols_by_file {
            debug!(
                "File: {} - {} symbols",
                file_path.display(),
                file_symbols.len()
            );
            for symbol in file_symbols {
                debug!("  {} ({:?})", symbol.name, symbol.kind);
            }
        }

        // Verify we found symbols from both files
        assert!(
            symbols.len() >= 3,
            "Should find at least 3 symbols across files, found {}",
            symbols.len()
        );

        // Check that we have symbols from both files
        let lib_symbols = symbols
            .iter()
            .filter(
                |s| match s.file_path.file_name().and_then(|name| name.to_str()) {
                    Some(filename) => filename == "lib.rs",
                    None => false,
                },
            )
            .count();
        let main_symbols = symbols
            .iter()
            .filter(
                |s| match s.file_path.file_name().and_then(|name| name.to_str()) {
                    Some(filename) => filename == "main.rs",
                    None => false,
                },
            )
            .count();

        assert!(lib_symbols > 0, "Should find symbols in lib.rs");
        assert!(main_symbols > 0, "Should find symbols in main.rs");

        info!(
            "Codebase indexing test completed successfully. Found {} symbols across {} files with {} file states tracked",
            symbols.len(),
            symbols_by_file.len(),
            codebase_state.file_states.len()
        );

        // clean up the index file
        std::fs::remove_file("./rua.index.json").ok();
    }
}
