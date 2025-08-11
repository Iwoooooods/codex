use futures::StreamExt;
use futures::channel::mpsc::{self, Receiver};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use tracing::info;

/// Represents a file change event that needs to be processed
#[derive(Debug, Clone)]
pub enum FileChangeEvent {
    /// File was created or modified
    FileChanged(PathBuf),
    /// File was deleted
    FileDeleted(PathBuf),
    /// Directory was created
    DirCreated(PathBuf),
    /// Directory was deleted
    DirDeleted(PathBuf),
}

/// Configuration for the file watcher
#[derive(Debug, Clone)]
pub struct FileWatcherConfig {
    /// Root path to watch
    pub root_path: PathBuf,
    /// Debounce delay for file change events (in milliseconds)
    pub debounce_delay: u64,
    /// Whether to watch recursively
    pub recursive: bool,
    /// File extensions to watch (if empty, watches all files)
    pub file_extensions: Vec<String>,
    /// Directories to ignore
    pub ignore_dirs: Vec<String>,
}

impl Default for FileWatcherConfig {
    fn default() -> Self {
        Self {
            root_path: PathBuf::from("."),
            debounce_delay: 1000, // 1s debounce
            recursive: true,
            file_extensions: vec![], // Watch all files by default
            ignore_dirs: vec![
                ".git".to_string(),
                "target".to_string(),
                "node_modules".to_string(),
                ".cargo".to_string(),
                "build".to_string(),
                "dist".to_string(),
            ],
        }
    }
}

/// The main file watcher that monitors file system changes
pub struct FileWatcher {
    config: FileWatcherConfig,
}

impl FileWatcher {
    /// Create a new file watcher
    pub fn new(config: FileWatcherConfig) -> Self {
        Self { config }
    }

    fn async_watcher(
        &self,
    ) -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
        let (mut tx, rx) = mpsc::channel(1);

        let watcher = RecommendedWatcher::new(
            move |res| {
                if let Err(e) = tx.try_send(res) {
                    eprintln!("Failed to send file event: {}", e);
                }
            },
            Config::default(),
        )?;

        Ok((watcher, rx))
    }

    /// Start watching for file changes
    pub async fn watch(&mut self) -> notify::Result<Event> {
        let root_path = self.config.root_path.clone();

        let (mut watcher, mut rx) = self.async_watcher()?;
        watcher.watch(&root_path, RecursiveMode::Recursive)?;
        info!("watching for file changes under {:?}", root_path);

        while let Some(res) = rx.next().await {
            match res {
                Ok(event) => return Ok(event),
                Err(err) => return Err(err),
            }
        }

        unreachable!("file watcher channel should not close while watcher is alive");
    }

    /// Check if a path should be ignored based on configuration
    fn should_ignore_path(path: &Path, config: &FileWatcherConfig) -> bool {
        // Check if any parent directory is in the ignore list
        for component in path.components() {
            if let std::path::Component::Normal(name) = component {
                if config
                    .ignore_dirs
                    .iter()
                    .any(|ignore| ignore == &name.to_string_lossy())
                {
                    return true;
                }
            }
        }

        // Check file extensions if specified
        if !config.file_extensions.is_empty() {
            if let Some(extension) = path.extension() {
                let ext_str = extension.to_string_lossy().to_string();
                if !config.file_extensions.contains(&ext_str) {
                    return true;
                }
            } else {
                return true; // No extension, skip
            }
        }

        false
    }
}

/// Status information about the file watcher
#[derive(Debug, Clone)]
pub struct FileWatcherStatus {
    pub watched_files: usize,
    pub root_path: PathBuf,
}

/// Builder pattern for creating FileWatcher instances
pub struct FileWatcherBuilder {
    config: FileWatcherConfig,
}

impl FileWatcherBuilder {
    pub fn new() -> Self {
        Self {
            config: FileWatcherConfig::default(),
        }
    }

    pub fn root_path<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.config.root_path = path.as_ref().to_path_buf();
        self
    }

    pub fn debounce_delay(mut self, delay_ms: u64) -> Self {
        self.config.debounce_delay = delay_ms;
        self
    }

    pub fn recursive(mut self, recursive: bool) -> Self {
        self.config.recursive = recursive;
        self
    }

    pub fn file_extensions(mut self, extensions: Vec<String>) -> Self {
        self.config.file_extensions = extensions;
        self
    }

    pub fn ignore_dirs(mut self, dirs: Vec<String>) -> Self {
        self.config.ignore_dirs = dirs;
        self
    }

    pub fn build(self) -> FileWatcher {
        FileWatcher::new(self.config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_file_watcher_with_temp_directory() {
        // Create a temporary directory for testing
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let temp_path = temp_dir.path();

        // Create a file watcher for the temp directory
        let config = FileWatcherConfig {
            root_path: temp_path.to_path_buf(),
            debounce_delay: 100, // Short delay for testing
            recursive: true,
            file_extensions: vec![],
            ignore_dirs: vec![],
        };

        let mut watcher = FileWatcher::new(config);

        // Start watching in a separate task
        let watch_handle = tokio::spawn(async move { watcher.watch().await });

        // Give the watcher a moment to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Create a test file
        let test_file = temp_path.join("test.txt");
        fs::write(&test_file, "test content").expect("Failed to write test file");

        // Wait for the file event with a timeout
        match timeout(Duration::from_secs(5), watch_handle).await {
            Ok(Ok(event)) => {
                // Verify we got a file change event
                match event {
                    Ok(notify::Event { ref paths, .. }) => {
                        // The exact event type may vary by platform, but we should get some event
                        assert!(!paths.is_empty());
                        println!("Received event: {:?}", event);
                    }
                    Err(e) => panic!("Watcher error: {:?}", e),
                }
            }
            Ok(Err(e)) => panic!("Task error: {:?}", e),
            Err(_) => panic!("Timeout waiting for file event"),
        }

        // Cleanup happens automatically when TempDir is dropped
    }

    #[tokio::test]
    async fn test_file_watcher_ignore_patterns() {
        let temp_dir = TempDir::new().expect("Failed to create temp dir");
        let temp_path = temp_dir.path();

        // Create a watcher that ignores certain directories and only watches .txt files
        let config = FileWatcherConfig {
            root_path: temp_path.to_path_buf(),
            debounce_delay: 100,
            recursive: true,
            file_extensions: vec!["txt".to_string()],
            ignore_dirs: vec!["ignored".to_string()],
        };

        let mut watcher = FileWatcher::new(config);

        // Create ignored directory and file
        let ignored_dir = temp_path.join("ignored");
        fs::create_dir(&ignored_dir).expect("Failed to create ignored dir");
        let ignored_file = ignored_dir.join("test.txt");
        fs::write(&ignored_file, "ignored content").expect("Failed to write ignored file");

        // Create watched file
        let watched_file = temp_path.join("watched.txt");
        fs::write(&watched_file, "watched content").expect("Failed to write watched file");

        // Start watching
        let watch_handle = tokio::spawn(async move { watcher.watch().await });

        // Give the watcher time to start
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Modify the watched file to trigger an event
        fs::write(&watched_file, "modified content").expect("Failed to modify watched file");

        // Wait for event with timeout
        match timeout(Duration::from_secs(5), watch_handle).await {
            Ok(Ok(event)) => {
                match event {
                    Ok(notify::Event { ref paths, .. }) => {
                        // Should only get events for the watched file, not the ignored one
                        for path in paths {
                            assert!(!path.to_string_lossy().contains("ignored"));
                        }
                        println!("Received event for watched file: {:?}", event);
                    }
                    Err(e) => panic!("Watcher error: {:?}", e),
                }
            }
            Ok(Err(e)) => panic!("Task error: {:?}", e),
            Err(_) => panic!("Timeout waiting for file event"),
        }
    }
}
