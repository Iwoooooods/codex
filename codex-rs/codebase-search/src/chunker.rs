use serde::Deserialize;
use serde::Serialize;
use std::path::PathBuf;
use tracing::debug;
use tracing::info;
use tracing::warn;

use crate::symbol::SupportedLanguage;
use crate::symbol::Symbol;
use crate::symbol::SymbolParser;

/// Represents a chunk of code ready for embedding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    /// The formatted text content to be embedded
    pub content: String,
    /// The file path where this chunk originates
    pub file_path: PathBuf,
    /// The starting line number of the chunk
    pub start_line: usize,
    /// The ending line number of the chunk
    pub end_line: usize,
    /// The original symbol name this chunk represents
    pub symbol_name: String,
    /// The kind of symbol this chunk represents
    pub symbol_kind: String,
    /// Context information (e.g., containing class/module)
    pub context: Option<String>,
    /// Metadata about the chunking process
    pub chunk_metadata: ChunkMetadata,
}

/// Metadata about how a chunk was created
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMetadata {
    /// Whether this chunk was split from a larger symbol
    pub is_split: bool,
    /// The original symbol size in lines before chunking
    pub original_size_lines: usize,
    /// The depth in the hierarchical chunking process
    pub chunk_depth: usize,
    /// Whether this is a container chunk (like an impl block)
    pub is_container: bool,
}

/// Configuration options for the chunking process
#[derive(Debug, Clone)]
pub struct ChunkingOptions {
    /// Maximum number of lines per chunk
    pub max_lines_per_chunk: usize,
    /// Minimum number of lines for a chunk to be considered valid
    pub min_lines_per_chunk: usize,
    /// Whether to include context metadata in chunk content
    pub include_metadata: bool,
    /// Maximum recursion depth for hierarchical chunking
    pub max_recursion_depth: usize,
}

impl Default for ChunkingOptions {
    fn default() -> Self {
        Self {
            max_lines_per_chunk: 200,
            min_lines_per_chunk: 5,
            include_metadata: true,
            max_recursion_depth: 5,
        }
    }
}

/// Hierarchical chunker that recursively breaks down large symbols
pub struct HierarchicalChunker {
    options: ChunkingOptions,
    parser: SymbolParser,
}

impl HierarchicalChunker {
    pub fn new(options: ChunkingOptions) -> Result<Self, anyhow::Error> {
        let parser = SymbolParser::new()?;
        Ok(Self { options, parser })
    }

    /// Create chunks from a list of symbols using hierarchical strategy
    pub fn chunk_symbols(&mut self, symbols: &[Symbol]) -> Result<Vec<CodeChunk>, anyhow::Error> {
        let mut all_chunks = Vec::new();

        info!(
            "Starting hierarchical chunking of {} symbols",
            symbols.len()
        );

        for symbol in symbols {
            let chunks = self.chunk_symbol_recursive(symbol, 0)?;
            all_chunks.extend(chunks);
        }

        info!(
            "Hierarchical chunking complete. Created {} chunks from {} symbols",
            all_chunks.len(),
            symbols.len()
        );
        Ok(all_chunks)
    }

    /// Recursively chunk a single symbol
    fn chunk_symbol_recursive(
        &mut self,
        symbol: &Symbol,
        depth: usize,
    ) -> Result<Vec<CodeChunk>, anyhow::Error> {
        debug!(
            "Chunking symbol '{}' ({:?}) at depth {}, size: {} lines",
            symbol.name,
            symbol.kind,
            depth,
            symbol.end_line - symbol.start_line + 1
        );

        // Check if we've hit maximum recursion depth
        if depth >= self.options.max_recursion_depth {
            warn!(
                "Hit maximum recursion depth for symbol '{}', creating single chunk",
                symbol.name
            );
            return Ok(vec![self.create_chunk_from_symbol(symbol, depth, false)]);
        }

        let symbol_size = symbol.end_line - symbol.start_line + 1;

        // If symbol is small enough, create a single chunk
        if symbol_size <= self.options.max_lines_per_chunk {
            debug!(
                "Symbol '{}' fits in single chunk ({} lines)",
                symbol.name, symbol_size
            );
            return Ok(vec![self.create_chunk_from_symbol(symbol, depth, false)]);
        }

        // Symbol is too large, try to break it down recursively
        debug!(
            "Symbol '{}' is too large ({} lines), attempting to break down",
            symbol.name, symbol_size
        );

        match self.try_recursive_chunking(symbol, depth) {
            Ok(sub_chunks) if !sub_chunks.is_empty() => {
                info!(
                    "Successfully broke down '{}' into {} sub-chunks",
                    symbol.name,
                    sub_chunks.len()
                );
                Ok(sub_chunks)
            }
            Ok(_) => {
                warn!(
                    "No sub-symbols found for '{}', creating single large chunk",
                    symbol.name
                );
                Ok(vec![self.create_chunk_from_symbol(symbol, depth, true)])
            }
            Err(e) => {
                warn!(
                    "Failed to break down '{}': {}, creating single chunk",
                    symbol.name, e
                );
                Ok(vec![self.create_chunk_from_symbol(symbol, depth, true)])
            }
        }
    }

    /// Try to recursively chunk a symbol by parsing its content for sub-symbols
    fn try_recursive_chunking(
        &mut self,
        symbol: &Symbol,
        depth: usize,
    ) -> Result<Vec<CodeChunk>, anyhow::Error> {
        // Determine language from file extension
        let extension = symbol
            .file_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        let language = SupportedLanguage::from_extension(extension)
            .ok_or_else(|| anyhow::anyhow!("Unsupported file extension: {}", extension))?;

        // Parse the symbol's content to find sub-symbols
        let parser =
            self.parser.parsers.get_mut(extension).ok_or_else(|| {
                anyhow::anyhow!("No parser available for extension: {}", extension)
            })?;

        let tree = parser
            .parse(&symbol.content, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse symbol content"))?;

        // Extract sub-symbols from the parsed content
        let sub_symbols = self
            .parser
            .extract_symbols(&tree, &symbol.content, &symbol.file_path, &language)
            .map_err(|e| anyhow::anyhow!("Failed to extract sub-symbols: {}", e))?;

        if sub_symbols.is_empty() {
            return Ok(vec![]);
        }

        debug!(
            "Found {} sub-symbols in '{}'",
            sub_symbols.len(),
            symbol.name
        );

        // Filter out symbols that are too small or are the same as the parent
        let valid_sub_symbols: Vec<_> = sub_symbols
            .into_iter()
            .filter(|sub_sym| {
                let sub_size = sub_sym.end_line - sub_sym.start_line + 1;
                sub_size >= self.options.min_lines_per_chunk && sub_sym.name != symbol.name
            })
            .collect();

        if valid_sub_symbols.is_empty() {
            return Ok(vec![]);
        }

        // Recursively chunk each valid sub-symbol
        let mut all_chunks = Vec::new();
        for sub_symbol in &valid_sub_symbols {
            let sub_chunks = self.chunk_symbol_recursive(sub_symbol, depth + 1)?;
            all_chunks.extend(sub_chunks);
        }

        // If we have container-level information (like impl blocks), create a container chunk
        if self.should_create_container_chunk(symbol, &valid_sub_symbols) {
            let container_chunk = self.create_container_chunk(symbol, depth, &valid_sub_symbols);
            all_chunks.insert(0, container_chunk);
        }

        Ok(all_chunks)
    }

    /// Determine if we should create a container chunk for organizational purposes
    fn should_create_container_chunk(&self, symbol: &Symbol, sub_symbols: &[Symbol]) -> bool {
        use crate::symbol::SymbolKind;

        matches!(
            symbol.kind,
            SymbolKind::Impl | SymbolKind::Module | SymbolKind::Struct | SymbolKind::Trait
        ) && !sub_symbols.is_empty()
            && sub_symbols.len() > 1
    }

    /// Create a container chunk that provides context for sub-symbols
    fn create_container_chunk(
        &self,
        symbol: &Symbol,
        depth: usize,
        sub_symbols: &[Symbol],
    ) -> CodeChunk {
        let content = if self.options.include_metadata {
            format!(
                "// File: {}, Container: {}, Kind: {:?}\n// Contains {} sub-symbols: {}\n\n{}",
                symbol.file_path.display(),
                symbol.name,
                symbol.kind,
                sub_symbols.len(),
                sub_symbols
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                self.extract_container_signature(symbol)
            )
        } else {
            self.extract_container_signature(symbol)
        };

        CodeChunk {
            content,
            file_path: symbol.file_path.clone(),
            start_line: symbol.start_line,
            end_line: symbol.end_line,
            symbol_name: symbol.name.clone(),
            symbol_kind: format!("{:?}", symbol.kind),
            context: symbol.context.clone(),
            chunk_metadata: ChunkMetadata {
                is_split: true,
                original_size_lines: symbol.end_line - symbol.start_line + 1,
                chunk_depth: depth,
                is_container: true,
            },
        }
    }

    /// Extract just the signature/header of a container symbol (without the full body)
    fn extract_container_signature(&self, symbol: &Symbol) -> String {
        // For now, just take the first few lines that likely contain the signature
        let lines: Vec<&str> = symbol.content.lines().collect();
        let signature_lines = std::cmp::min(10, lines.len());

        let mut signature = lines[..signature_lines].join("\n");

        // If we truncated, indicate it
        if signature_lines < lines.len() {
            signature.push_str("\n\n// ... (content continues) ...");
        }

        signature
    }

    /// Create a single chunk from a symbol
    fn create_chunk_from_symbol(&self, symbol: &Symbol, depth: usize, is_split: bool) -> CodeChunk {
        let content = if self.options.include_metadata {
            format!(
                "// File: {}, Symbol: {}, Kind: {:?}\n{}\n, Content: {}\n",
                symbol.file_path.display(),
                symbol.name,
                symbol.kind,
                symbol
                    .context
                    .as_ref()
                    .map(|ctx| format!("// Context: {ctx}"))
                    .unwrap_or_default(),
                symbol.content
            )
        } else {
            symbol.content.clone()
        };

        CodeChunk {
            content,
            file_path: symbol.file_path.clone(),
            start_line: symbol.start_line,
            end_line: symbol.end_line,
            symbol_name: symbol.name.clone(),
            symbol_kind: format!("{:?}", symbol.kind),
            context: symbol.context.clone(),
            chunk_metadata: ChunkMetadata {
                is_split,
                original_size_lines: symbol.end_line - symbol.start_line + 1,
                chunk_depth: depth,
                is_container: false,
            },
        }
    }
}

/// Simple chunking strategy that creates one chunk per symbol
pub fn create_simple_chunks_from_symbols(symbols: &[Symbol]) -> Vec<CodeChunk> {
    symbols
        .iter()
        .map(|symbol| {
            let content = format!(
                "// File: {}, Symbol: {}, Kind: {:?}\n\n{}",
                symbol.file_path.display(),
                symbol.name,
                symbol.kind,
                symbol.content
            );

            CodeChunk {
                content,
                file_path: symbol.file_path.clone(),
                start_line: symbol.start_line,
                end_line: symbol.end_line,
                symbol_name: symbol.name.clone(),
                symbol_kind: format!("{:?}", symbol.kind),
                context: symbol.context.clone(),
                chunk_metadata: ChunkMetadata {
                    is_split: false,
                    original_size_lines: symbol.end_line - symbol.start_line + 1,
                    chunk_depth: 0,
                    is_container: false,
                },
            }
        })
        .collect()
}

/// Index a codebase and create chunks ready for embedding using hierarchical strategy
pub async fn index_codebase<P: AsRef<std::path::Path>>(
    root_path: P,
    chunking_options: ChunkingOptions,
) -> Result<Vec<crate::embedding::EmbeddedChunk>, anyhow::Error> {
    // 1. Extract symbols
    let symbols = crate::symbol::parse_codebase(root_path)?;

    // 2. Create chunker and process symbols
    let mut chunker = HierarchicalChunker::new(chunking_options)?;
    let chunks = chunker.chunk_symbols(&symbols)?;

    // 3. Embed chunks
    let config = crate::embedding::EmbeddingConfig::default();
    let client = crate::embedding::EmbeddingClient::new(config)?;
    let embedded_chunks = client.embed_chunks(&chunks).await?;
    Ok(embedded_chunks)
}
