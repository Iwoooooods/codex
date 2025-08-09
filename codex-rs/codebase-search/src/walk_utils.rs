use ignore::WalkBuilder;
use std::path::Path;
use tracing::debug;

/// Creates a WalkBuilder configured with common ignore patterns for codebase analysis
/// This function sets up directory walking that respects .gitignore files and excludes
/// common build and temporary directories that shouldn't be analyzed.
pub fn create_codebase_walker<P: AsRef<Path>>(root_path: P) -> ignore::WalkBuilder {
    let mut builder = WalkBuilder::new(root_path.as_ref());

    // Respect .gitignore files
    builder.git_ignore(true);

    // Respect .ignore files (used by ripgrep and other tools)
    builder.ignore(true);

    // Don't follow symlinks to avoid infinite loops
    builder.follow_links(false);

    // Add common ignore patterns for directories that shouldn't be indexed
    builder.add_custom_ignore_filename(".codexignore");

    // Built-in ignore patterns for common build/cache directories
    // These are commonly excluded directories in development projects
    let ignore_patterns = vec![
        "target/",        // Rust build directory
        "build/",         // General build directory
        "dist/",          // Distribution directory
        "node_modules/",  // Node.js dependencies
        ".git/",          // Git metadata
        ".svn/",          // SVN metadata
        ".hg/",           // Mercurial metadata
        "__pycache__/",   // Python bytecode cache
        ".pytest_cache/", // Pytest cache
        ".mypy_cache/",   // MyPy cache
        ".venv/",         // Python virtual environment
        "venv/",          // Python virtual environment
        ".env/",          // Environment directory
        "coverage/",      // Coverage reports
        ".coverage/",     // Coverage data
        ".nyc_output/",   // NYC coverage output
        ".cache/",        // General cache directory
        "tmp/",           // Temporary files
        "temp/",          // Temporary files
        ".tmp/",          // Hidden temporary files
        ".DS_Store",      // macOS metadata files
        "Thumbs.db",      // Windows thumbnail cache
    ];

    // Add these as exclude patterns using override builder
    let mut override_builder = ignore::overrides::OverrideBuilder::new(root_path.as_ref());
    for pattern in ignore_patterns {
        // The `!` prefix indicates an exclude pattern
        let exclude_pattern = format!("!{pattern}");
        if let Err(e) = override_builder.add(&exclude_pattern) {
            debug!("Failed to add ignore pattern '{}': {}", pattern, e);
        }
    }

    if let Ok(overrides) = override_builder.build() {
        builder.overrides(overrides);
    }

    debug!(
        "Created codebase walker for: {}",
        root_path.as_ref().display()
    );

    builder
}

/// Walks through a codebase directory and calls the provided closure for each file
/// This is a simplified interface that handles the common pattern of walking files
/// while respecting ignore patterns.
pub fn walk_codebase_files<P, F>(root_path: P, mut file_handler: F) -> Result<(), anyhow::Error>
where
    P: AsRef<Path>,
    F: FnMut(&Path) -> Result<bool, anyhow::Error>, // Return false to stop walking
{
    let walker = create_codebase_walker(root_path.as_ref());

    for entry in walker.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                debug!("Skipping entry due to error: {}", err);
                continue;
            }
        };

        let path = entry.path();

        // Only process files, skip directories
        if !path.is_file() {
            continue;
        }

        // Call the handler and check if we should continue
        match file_handler(path) {
            Ok(true) => continue, // Continue walking
            Ok(false) => break,   // Stop walking
            Err(e) => {
                debug!("Error processing file {}: {}", path.display(), e);
                continue; // Skip this file but continue walking
            }
        }
    }

    Ok(())
}

/// Checks if a file extension is supported for code analysis
pub fn is_supported_file_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("rs" | "py" | "go")
    )
}
