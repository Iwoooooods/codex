use std::collections::HashMap;

use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodebaseState {
    pub file_states: HashMap<String, FileState>,
}

impl CodebaseState {
    pub fn to_file(&self) -> Result<(), anyhow::Error> {
        let file_path = "./rua.index.json";
        let file_content = serde_json::to_string_pretty(self)?;
        std::fs::write(file_path, file_content)?;
        Ok(())
    }

    pub fn from_file() -> Result<Self, anyhow::Error> {
        let file_path = "./rua.index.json";
        let file_content = std::fs::read_to_string(file_path)?;
        let codebase_state: CodebaseState = serde_json::from_str(&file_content)?;
        Ok(codebase_state)
    }
}

/// FileState is used to track the state of a file
/// if its not found -> new file
/// if its found -> check if the content is the same
///     if the last_modified is different from the one in your file system -> maybe modified
///         if the content_md5 is different -> definitely modified
///     else -> unchanged
/// use a set to track the files that are seen
/// if the file is not in the set -> deleted
/// then we get added_files, modified_files, deleted_files
/// we will use them to update the vector db
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
pub struct FileState {
    pub content_md5: String,
    pub last_modified: u64,
}

impl FileState {
    pub fn new(file_path: String, last_modified: u64) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(&file_path)
            .map_err(|e| anyhow::anyhow!("Failed to read file '{}': {}", file_path, e))?;
        let content_md5 = format!("{:x}", md5::compute(content));
        Ok(Self {
            content_md5,
            last_modified,
        })
    }
}
