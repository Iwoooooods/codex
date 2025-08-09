use anyhow::Result;
use clap::Parser;
use clap::Subcommand;
use codebase_search::chunker::ChunkingOptions;
use codebase_search::chunker::chunk_codebase;
use codebase_search::symbol::SymbolKind;
use codebase_search::symbol::SymbolParser;
use codebase_search::symbol::parse_codebase;
use codebase_search::vector_db::restore_session;
use std::path::PathBuf;
use tracing::info;
use tracing::warn;

/// A CLI tool for parsing and analyzing codebase symbols
#[derive(Parser)]
#[command(name = "codebase-search")]
#[command(about = "A tool for extracting and analyzing symbols from codebases")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Parse a single file and extract symbols
    ParseFile {
        /// Path to the file to parse
        #[arg(value_name = "FILE")]
        file_path: PathBuf,

        /// Output format (json, pretty, summary)
        #[arg(short = 'f', long, default_value = "pretty")]
        format: String,
    },
    /// Parse an entire codebase directory
    ParseCodebase {
        /// Path to the codebase directory
        #[arg(value_name = "DIRECTORY")]
        directory: PathBuf,

        /// Output format (json, pretty, summary)
        #[arg(short = 'f', long, default_value = "summary")]
        format: String,

        /// Filter by symbol kind (function, struct, class, etc.)
        #[arg(short = 'k', long)]
        kind_filter: Option<String>,

        /// Filter by file extension
        #[arg(short = 'e', long)]
        extension_filter: Option<String>,
    },
    /// Chunk a codebase for embedding (extract symbols and create chunks)
    ChunkCodebase {
        /// Path to the codebase directory
        #[arg(value_name = "DIRECTORY")]
        directory: PathBuf,

        /// Output format (json, pretty, summary)
        #[arg(short = 'f', long, default_value = "summary")]
        format: String,

        /// Maximum lines per chunk
        #[arg(long, default_value = "200")]
        max_lines: usize,

        /// Minimum lines per chunk
        #[arg(long, default_value = "5")]
        min_lines: usize,

        /// Include metadata in chunk content
        #[arg(long)]
        include_metadata: bool,

        /// Maximum recursion depth for hierarchical chunking
        #[arg(long, default_value = "5")]
        max_depth: usize,
    },
    /// Initialize or update codebase index in vector database (automatically detects changes)
    IndexCodebase {
        /// Path to the codebase directory
        #[arg(value_name = "DIRECTORY")]
        directory: PathBuf,
    },
    /// Search the indexed codebase using semantic similarity
    SearchCodebase {
        /// Search query
        #[arg(value_name = "QUERY")]
        query: String,

        /// Path to the codebase directory (for collection identification)
        #[arg(value_name = "DIRECTORY")]
        directory: PathBuf,

        /// Number of results to return
        #[arg(short = 'n', long, default_value = "10")]
        limit: usize,

        /// Minimum similarity score (0.0 to 1.0)
        #[arg(long, default_value = "0.7")]
        min_score: f32,
    },
    /// Show supported languages and file extensions
    Languages,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    let log_level = if cli.verbose {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    tracing_subscriber::fmt().with_max_level(log_level).init();

    match cli.command {
        Commands::ParseFile { file_path, format } => {
            parse_single_file(file_path, &format)?;
        }
        Commands::ParseCodebase {
            directory,
            format,
            kind_filter,
            extension_filter,
        } => {
            parse_codebase_directory(directory, &format, kind_filter, extension_filter)?;
        }
        Commands::ChunkCodebase {
            directory,
            format,
            max_lines,
            min_lines,
            include_metadata,
            max_depth,
        } => {
            chunk_codebase_command(
                directory,
                &format,
                max_lines,
                min_lines,
                include_metadata,
                max_depth,
            )
            .await?;
        }
        Commands::IndexCodebase { directory } => {
            index_codebase_command(directory).await?;
        }
        Commands::SearchCodebase {
            query,
            directory,
            limit,
            min_score,
        } => {
            search_codebase_command(query, directory, limit, min_score).await?;
        }
        Commands::Languages => {
            show_supported_languages();
        }
    }

    Ok(())
}

fn parse_single_file(file_path: PathBuf, format: &str) -> Result<()> {
    info!("Parsing file: {}", file_path.display());

    let mut parser = SymbolParser::new()?;
    let symbols = parser
        .parse_file(&file_path)
        .map_err(|e| anyhow::anyhow!("Failed to parse file '{}': {}", file_path.display(), e))?;

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&symbols)?;
            println!("{json}");
        }
        "pretty" => {
            println!("=== Symbols in {} ===", file_path.display());
            print_symbols_pretty(&symbols);
        }
        "summary" => {
            print_symbols_summary(&symbols, Some(&file_path));
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported format: {format}. Use 'json', 'pretty', or 'summary'"
            ));
        }
    }

    Ok(())
}

fn parse_codebase_directory(
    directory: PathBuf,
    format: &str,
    kind_filter: Option<String>,
    extension_filter: Option<String>,
) -> Result<()> {
    info!("Parsing codebase: {}", directory.display());

    let symbols = parse_codebase(&directory)?;

    // Apply filters
    let filtered_symbols: Vec<_> = symbols
        .into_iter()
        .filter(|symbol| {
            // Filter by kind if specified
            if let Some(ref kind_str) = kind_filter {
                let kind_matches = match kind_str.to_lowercase().as_str() {
                    "function" => matches!(symbol.kind, SymbolKind::Function),
                    "method" => matches!(symbol.kind, SymbolKind::Method),
                    "struct" => matches!(symbol.kind, SymbolKind::Struct),
                    "class" => matches!(symbol.kind, SymbolKind::Class),
                    "enum" => matches!(symbol.kind, SymbolKind::Enum),
                    "trait" => matches!(symbol.kind, SymbolKind::Trait),
                    "interface" => matches!(symbol.kind, SymbolKind::Interface),
                    "impl" => matches!(symbol.kind, SymbolKind::Impl),
                    "module" => matches!(symbol.kind, SymbolKind::Module),
                    "constant" => matches!(symbol.kind, SymbolKind::Constant),
                    "variable" => matches!(symbol.kind, SymbolKind::Variable),
                    "type" => matches!(symbol.kind, SymbolKind::Type),
                    _ => {
                        warn!("Unknown symbol kind filter: {kind_str}");
                        true
                    }
                };
                if !kind_matches {
                    return false;
                }
            }

            // Filter by extension if specified
            if let Some(ref ext) = extension_filter {
                if let Some(file_ext) = symbol.file_path.extension().and_then(|e| e.to_str()) {
                    if file_ext != ext {
                        return false;
                    }
                }
            }

            true
        })
        .collect();

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&filtered_symbols)?;
            println!("{json}");
        }
        "pretty" => {
            println!("=== Symbols in {} ===", directory.display());
            print_symbols_pretty(&filtered_symbols);
        }
        "summary" => {
            print_symbols_summary(&filtered_symbols, None);
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported format: {format}. Use 'json', 'pretty', or 'summary'"
            ));
        }
    }

    Ok(())
}

async fn chunk_codebase_command(
    directory: PathBuf,
    format: &str,
    max_lines: usize,
    min_lines: usize,
    include_metadata: bool,
    max_depth: usize,
) -> Result<()> {
    info!("Chunking codebase: {}", directory.display());

    let chunking_options = ChunkingOptions {
        max_lines_per_chunk: max_lines,
        min_lines_per_chunk: min_lines,
        include_metadata,
        max_recursion_depth: max_depth,
    };

    let embedded_chunks = chunk_codebase(&directory, chunking_options).await?;
    let chunks: Vec<_> = embedded_chunks.into_iter().map(|ec| ec.chunk).collect();

    match format {
        "json" => {
            let json = serde_json::to_string_pretty(&chunks)?;
            println!("{json}");
        }
        "pretty" => {
            println!("=== Chunks in {} ===", directory.display());
            print_chunks_pretty(&chunks);
        }
        "summary" => {
            print_chunks_summary(&chunks);
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Unsupported format: {format}. Use 'json', 'pretty', or 'summary'"
            ));
        }
    }

    Ok(())
}

async fn index_codebase_command(directory: PathBuf) -> Result<()> {
    // Canonicalize the directory path to convert relative paths to absolute paths
    let canonical_directory = directory
        .canonicalize()
        .unwrap_or_else(|_| directory.clone());

    println!("🔍 Analyzing codebase: {}", canonical_directory.display());
    println!(
        "📊 This will automatically detect and process only changed files for optimal performance."
    );

    // restore_session intelligently handles both initial indexing and incremental updates
    restore_session(&canonical_directory).await?;

    println!("✅ Codebase indexed successfully into vector database!");
    println!(
        "🗂️  Collection available for: {}",
        canonical_directory.display()
    );
    println!("💡 Use 'search-codebase' command to query the indexed code.");
    Ok(())
}

async fn search_codebase_command(
    query: String,
    _directory: PathBuf,
    limit: usize,
    min_score: f32,
) -> Result<()> {
    use codebase_search::retriever::search_codebase;

    info!("Searching indexed codebase for query: {}", query);

    println!("🔍 Searching codebase for: \"{query}\"");
    println!("🎯 Limit: {limit}, Min score: {min_score:.2}");
    println!();

    match search_codebase(query, limit, min_score).await {
        Ok(results) => {
            if results.is_empty() {
                println!("❌ No results found matching your query.");
                println!("💡 Try:");
                println!("   - Using different keywords");
                println!("   - Lowering the minimum score (current: {min_score:.2})");
                println!("   - Checking if the codebase is indexed with 'index-codebase' command");
            } else {
                println!("✅ Found {} results:", results.len());
                println!();

                for (i, result) in results.iter().enumerate() {
                    print_search_result(i + 1, result);
                    if i < results.len() - 1 {
                        println!("{}", "─".repeat(80));
                    }
                }

                println!();
                println!(
                    "🎯 Search completed. Showing {} results with score >= {:.2}",
                    results.len(),
                    min_score
                );
            }
        }
        Err(e) => {
            eprintln!("❌ Search failed: {e}");
            eprintln!("💡 Make sure:");
            eprintln!("   - Qdrant is running on localhost:6334");
            eprintln!("   - The codebase is indexed (run 'index-codebase' first)");
            eprintln!("   - The directory path is correct");
            return Err(e);
        }
    }

    Ok(())
}

fn print_symbols_pretty(symbols: &[codebase_search::symbol::Symbol]) {
    use std::collections::HashMap;

    // Group symbols by file
    let mut symbols_by_file: HashMap<&PathBuf, Vec<&codebase_search::symbol::Symbol>> =
        HashMap::new();
    for symbol in symbols {
        symbols_by_file
            .entry(&symbol.file_path)
            .or_default()
            .push(symbol);
    }

    for (file_path, file_symbols) in symbols_by_file {
        println!("\n📁 {}", file_path.display());
        println!("   {} symbols found", file_symbols.len());

        for symbol in file_symbols {
            let kind_emoji = match symbol.kind {
                SymbolKind::Function => "🔧",
                SymbolKind::Method => "⚙️",
                SymbolKind::Struct => "🏗️",
                SymbolKind::Class => "🏛️",
                SymbolKind::Enum => "🎯",
                SymbolKind::Trait => "🤝",
                SymbolKind::Interface => "🔌",
                SymbolKind::Impl => "🔗",
                SymbolKind::Module => "📦",
                SymbolKind::Constant => "🔒",
                SymbolKind::Variable => "📊",
                SymbolKind::Type => "🏷️",
            };

            let context_info = symbol
                .context
                .as_ref()
                .map(|c| format!(" (in {c})"))
                .unwrap_or_default();

            println!(
                "   {kind_emoji} {} {:?} at {}:{}-{}:{}{context_info}",
                symbol.name,
                symbol.kind,
                symbol.start_line,
                symbol.start_column,
                symbol.end_line,
                symbol.end_column
            );
        }
    }
}

fn print_chunks_pretty(chunks: &[codebase_search::chunker::CodeChunk]) {
    use std::collections::HashMap;

    // Group chunks by file
    let mut chunks_by_file: HashMap<&PathBuf, Vec<&codebase_search::chunker::CodeChunk>> =
        HashMap::new();
    for chunk in chunks {
        chunks_by_file
            .entry(&chunk.file_path)
            .or_default()
            .push(chunk);
    }

    for (file_path, file_chunks) in chunks_by_file {
        println!("\n📁 {}", file_path.display());
        println!("   {} chunks found", file_chunks.len());

        for chunk in file_chunks {
            let kind_emoji = match chunk.symbol_kind.as_str() {
                "Function" => "🔧",
                "Method" => "⚙️",
                "Struct" => "🏗️",
                "Class" => "🏛️",
                "Enum" => "🎯",
                "Trait" => "🤝",
                "Interface" => "🔌",
                "Impl" => "🔗",
                "Module" => "📦",
                "Constant" => "🔒",
                "Variable" => "📊",
                "Type" => "🏷️",
                _ => "📄",
            };

            let content_preview = if chunk.content.len() > 100 {
                format!("{}...", &chunk.content[..100])
            } else {
                chunk.content.clone()
            };

            println!(
                "   {} {} ({}) at {}:{} (depth: {}, {} lines) - {}",
                kind_emoji,
                chunk.symbol_name,
                chunk.symbol_kind,
                chunk.start_line,
                chunk.end_line,
                chunk.chunk_metadata.chunk_depth,
                chunk.chunk_metadata.original_size_lines,
                content_preview.replace('\n', " ")
            );
        }
    }
}

fn print_chunks_summary(chunks: &[codebase_search::chunker::CodeChunk]) {
    use std::collections::HashMap;

    println!("=== Chunking Summary ===");

    // Count chunks by file and kind
    let mut file_counts: HashMap<PathBuf, usize> = HashMap::new();
    let mut kind_counts: HashMap<String, usize> = HashMap::new();
    let mut depth_counts: HashMap<usize, usize> = HashMap::new();
    let mut total_lines = 0;

    for chunk in chunks {
        *file_counts.entry(chunk.file_path.clone()).or_insert(0) += 1;
        *kind_counts.entry(chunk.symbol_kind.clone()).or_insert(0) += 1;
        *depth_counts
            .entry(chunk.chunk_metadata.chunk_depth)
            .or_insert(0) += 1;
        total_lines += chunk.chunk_metadata.original_size_lines;
    }

    println!("Total chunks found: {}", chunks.len());
    println!("Files processed: {}", file_counts.len());
    println!("Total lines: {total_lines}");

    if !kind_counts.is_empty() {
        println!("\n🏷️  By Symbol Kind:");
        for (kind, count) in kind_counts {
            println!("   {kind}: {count}");
        }
    }

    if !depth_counts.is_empty() {
        println!("\n📊 By Chunk Depth:");
        for (depth, count) in depth_counts {
            println!("   Depth {depth}: {count} chunks");
        }
    }

    if !file_counts.is_empty() {
        println!("\n📁 Files with most chunks:");
        let mut files: Vec<_> = file_counts.iter().collect();
        files.sort_by(|a, b| b.1.cmp(a.1));

        for (file_path, count) in files.iter().take(5) {
            println!("   {} - {} chunks", file_path.display(), count);
        }
    }
}

fn print_symbols_summary(
    symbols: &[codebase_search::symbol::Symbol],
    single_file: Option<&PathBuf>,
) {
    use std::collections::HashMap;

    if let Some(file_path) = single_file {
        println!("=== Summary for {} ===", file_path.display());
    } else {
        println!("=== Codebase Summary ===");
    }

    // Count symbols by kind
    let mut kind_counts: HashMap<SymbolKind, usize> = HashMap::new();
    let mut file_counts: HashMap<PathBuf, usize> = HashMap::new();
    let mut language_counts: HashMap<String, usize> = HashMap::new();

    for symbol in symbols {
        *kind_counts.entry(symbol.kind.clone()).or_insert(0) += 1;
        *file_counts.entry(symbol.file_path.clone()).or_insert(0) += 1;

        if let Some(ext) = symbol.file_path.extension().and_then(|e| e.to_str()) {
            *language_counts.entry(ext.to_string()).or_insert(0) += 1;
        }
    }

    println!("Total symbols found: {}", symbols.len());
    println!("Files processed: {}", file_counts.len());

    if !language_counts.is_empty() {
        println!("\n📋 By Language:");
        for (lang, count) in language_counts {
            println!("   .{lang}: {count} symbols");
        }
    }

    println!("\n🏷️  By Symbol Kind:");
    for (kind, count) in kind_counts {
        println!("   {kind:?}: {count}");
    }

    if single_file.is_none() && file_counts.len() > 1 {
        println!("\n📁 Files with most symbols:");
        let mut files: Vec<_> = file_counts.iter().collect();
        files.sort_by(|a, b| b.1.cmp(a.1));

        for (file_path, count) in files.iter().take(5) {
            println!("   {} - {} symbols", file_path.display(), count);
        }
    }
}

fn show_supported_languages() {
    println!("=== Supported Languages ===");
    println!("🦀 Rust (.rs)");
    println!("   - Functions, Methods, Structs, Enums, Traits, Impls, Modules, Constants");

    println!("🐍 Python (.py)");
    println!("   - Functions, Methods, Classes");

    println!("🐹 Go (.go)");
    println!("   - Functions, Methods, Types (structs/interfaces), Constants, Variables");

    println!("\n=== Usage Examples ===");
    println!("Parse a single Rust file:");
    println!("  codebase-search parse-file src/main.rs");

    println!("\nParse entire codebase with summary:");
    println!("  codebase-search parse-codebase . --format summary");

    println!("\nFilter by symbol kind:");
    println!("  codebase-search parse-codebase . --kind-filter function");

    println!("\nFilter by file extension:");
    println!("  codebase-search parse-codebase . --extension-filter rs");

    println!("\nChunk a codebase for embedding:");
    println!("  codebase-search chunk-codebase . --max-lines 150 --include-metadata");

    println!("\nIndex a codebase into vector database (with smart incremental updates):");
    println!("  codebase-search index-codebase .");

    println!("\nSearch the indexed codebase:");
    println!("  codebase-search search-codebase \"authentication logic\" . --limit 5");

    println!("\nOutput as JSON:");
    println!("  codebase-search parse-file src/lib.rs --format json");
}

fn print_search_result(index: usize, result: &codebase_search::retriever::SearchResult) {
    let chunk = &result.chunk;

    let kind_emoji = match chunk.symbol_kind.as_str() {
        "Function" => "🔧",
        "Method" => "⚙️",
        "Struct" => "🏗️",
        "Class" => "🏛️",
        "Enum" => "🎯",
        "Trait" => "🤝",
        "Interface" => "🔌",
        "Impl" => "🔗",
        "Module" => "📦",
        "Constant" => "🔒",
        "Variable" => "📊",
        "Type" => "🏷️",
        _ => "📄",
    };

    // Header with result index, symbol info, and score
    println!(
        "{}. {} {} {} (Score: {:.3})",
        index, kind_emoji, chunk.symbol_kind, chunk.symbol_name, result.score
    );

    // File and location info
    println!(
        "   📁 {}:{}-{}",
        chunk.file_path.display(),
        chunk.start_line,
        chunk.end_line
    );

    // Context if available
    if let Some(ref context) = chunk.context {
        println!("   🗂️  Context: {context}");
    }

    // Additional metadata
    println!(
        "   📊 Chunk: depth {}, {} lines{}",
        chunk.chunk_metadata.chunk_depth,
        chunk.chunk_metadata.original_size_lines,
        if chunk.chunk_metadata.is_split {
            " (split)"
        } else {
            ""
        }
    );

    // Content preview (limit to first few lines and max characters)
    let content_lines: Vec<&str> = chunk.content.lines().collect();
    let preview_lines = if content_lines.len() > 5 {
        5
    } else {
        content_lines.len()
    };

    println!("   📝 Content preview:");
    for line in content_lines.iter().take(preview_lines) {
        let trimmed_line = if line.len() > 100 {
            format!("{}...", &line[..100])
        } else {
            line.to_string()
        };
        println!("      {trimmed_line}");
    }

    if content_lines.len() > preview_lines {
        println!(
            "      ... ({} more lines)",
            content_lines.len() - preview_lines
        );
    }

    println!();
}
