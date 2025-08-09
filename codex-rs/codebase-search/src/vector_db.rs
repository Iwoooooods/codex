use crate::walk_utils::is_supported_file_extension;
use crate::walk_utils::walk_codebase_files;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::LazyLock;
use tracing::debug;
use tracing::info;
use tracing::warn;

use serde_json::json;

use crate::chunker::ChunkingOptions;
use crate::chunker::chunk_codebase;
use crate::chunker::chunk_codefile;
use crate::embedding::QDRANT_EMBEDDING_DIMENSION;
use crate::file_state::CodebaseState;
use crate::file_state::FileState;
use crate::symbol::get_file_metadata;
use qdrant_client::Payload;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::Condition;
use qdrant_client::qdrant::CreateCollectionBuilder;
use qdrant_client::qdrant::DeletePointsBuilder;
use qdrant_client::qdrant::Distance;
use qdrant_client::qdrant::Filter;
use qdrant_client::qdrant::PointStruct;
use qdrant_client::qdrant::UpsertPointsBuilder;
use qdrant_client::qdrant::VectorParamsBuilder;
use sha2::Digest;
use sha2::Sha256;

/// Generate a deterministic point ID from file path and chunk position
/// This ensures we can properly upsert points for the same chunk across updates
/// Returns a deterministic UUID-v5-like string that Qdrant accepts
fn generate_point_id(
    file_path: &str,
    start_line: usize,
    end_line: usize,
    symbol_name: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(file_path.as_bytes());
    hasher.update(start_line.to_string().as_bytes());
    hasher.update(end_line.to_string().as_bytes());
    hasher.update(symbol_name.as_bytes());
    let hash = hasher.finalize();

    // Format as a UUID-like string that Qdrant will accept
    // Take first 32 hex chars and format as UUID: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
    let hex_str = format!("{hash:x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex_str[0..8],
        &hex_str[8..12],
        &hex_str[12..16],
        &hex_str[16..20],
        &hex_str[20..32]
    )
}

pub(crate) static QDRANT_CLIENT: LazyLock<Arc<Qdrant>> =
    LazyLock::new(|| match Qdrant::from_url("http://localhost:6334").build() {
        Ok(client) => Arc::new(client),
        Err(e) => panic!("Failed to create Qdrant client: {e}"),
    });

/// Generate a unique collection ID from a root path using SHA-256 hashing
/// This creates a deterministic, unique identifier that's safe for use as a collection name
/// The collection ID will be the same for the same root path across different sessions
pub(crate) fn generate_collection_id<P: AsRef<Path>>(root_path: P) -> String {
    let mut hasher = Sha256::new();

    hasher.update(root_path.as_ref().to_string_lossy().as_bytes());

    let hash = hasher.finalize();
    let hash_str = format!("{hash:x}");

    // Take the first 16 characters of the hash to keep it reasonably short
    // while still maintaining uniqueness
    format!("rua_{}", &hash_str[..16])
}

/// Helper function to clean up a collection when operations fail
/// This is used by both init_session and restore_session
async fn cleanup_collection(collection_id: &str, reason: &str) {
    warn!("Cleaning up collection {collection_id} due to error: {reason}");
    if let Err(cleanup_err) = QDRANT_CLIENT.delete_collection(collection_id).await {
        warn!("Failed to cleanup collection {collection_id} after error: {cleanup_err}");
    } else {
        info!("Successfully cleaned up collection {collection_id}");
    }
}

// New helper to collect supported file states under a root path
fn collect_supported_file_states<P: AsRef<Path>>(
    root_path: P,
) -> Result<HashMap<String, FileState>, anyhow::Error> {
    let mut file_states = HashMap::new();
    let root_path = root_path.as_ref();

    walk_codebase_files(root_path, |path| {
        // Only process supported file types
        if !is_supported_file_extension(path) {
            return Ok(true); // Continue walking
        }

        let file_path_str = path
            .strip_prefix(root_path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Get file metadata (last modified timestamp)
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

        file_states.insert(file_path_str, file_state);
        Ok(true) // Continue walking
    })?;

    Ok(file_states)
}

/// init_vector_db creates a new collection for the first time launched a project
/// it will generate a unique collection id based on the root path
/// for each project, we maintain a state file to track the modification of the project
/// it may be like index.json:
/// {
///     "src/main.rs": {
///      "content_hash": "sha256-of-content-v1",
///       "last_modified": 1678886400
///     },
///     "src/utils.rs": {
///       "content_hash": "sha256-of-content-v1",
///       "last_modified": 1678886401
///     }
/// }
pub async fn init_session<P: AsRef<Path>>(root_path: P) -> Result<(), anyhow::Error> {
    let collection_id = generate_collection_id(root_path.as_ref());

    // Check if collection already exists and delete it if it does
    // This handles the case where a previous init failed partway through
    match QDRANT_CLIENT.collection_info(&collection_id.clone()).await {
        Ok(_) => {
            warn!(
                "Collection {} already exists, deleting it before recreating",
                collection_id
            );
            QDRANT_CLIENT
                .delete_collection(&collection_id)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to delete existing collection {}: {}",
                        collection_id,
                        e
                    )
                })?;
        }
        Err(_) => {
            // Collection doesn't exist, which is expected for a new init
            debug!(
                "Collection {} doesn't exist, proceeding with creation",
                collection_id
            );
        }
    }

    // Create a new collection
    QDRANT_CLIENT
        .create_collection(
            CreateCollectionBuilder::new(collection_id.clone()).vectors_config(
                VectorParamsBuilder::new(QDRANT_EMBEDDING_DIMENSION as u64, Distance::Cosine),
            ),
        )
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create collection {}: {}", collection_id, e))?;

    info!("Created collection: {}", collection_id);

    // From this point on, if anything fails, we need to clean up the collection
    let collection_id_for_cleanup = collection_id.clone();

    // Index the project
    let opts = ChunkingOptions::default();
    let chunks = match chunk_codebase(root_path.as_ref(), opts).await {
        Ok(chunks) => chunks,
        Err(e) => {
            let error_msg = format!("Failed to chunk codebase: {e}");
            cleanup_collection(&collection_id_for_cleanup, &error_msg).await;
            return Err(anyhow::anyhow!(error_msg));
        }
    };

    // Convert chunks to points with metadata
    let points = chunks
        .into_iter()
        .map(|chunk| {
            let file_path_relative = chunk
                .chunk
                .file_path
                .strip_prefix(root_path.as_ref())
                .unwrap_or(&chunk.chunk.file_path)
                .to_string_lossy()
                .to_string();

            let payload = match Payload::try_from(json!({
                "file_path": file_path_relative.clone(),
                "start_line": chunk.chunk.start_line,
                "end_line": chunk.chunk.end_line,
                "symbol_name": chunk.chunk.symbol_name.clone(),
                "symbol_kind": chunk.chunk.symbol_kind.clone(),
                "is_container": chunk.chunk.chunk_metadata.is_container,
                "original_size_lines": chunk.chunk.chunk_metadata.original_size_lines,
                "is_split": chunk.chunk.chunk_metadata.is_split,
                "chunk_depth": chunk.chunk.chunk_metadata.chunk_depth,
                "context": chunk.chunk.context.clone(),
                "content": chunk.chunk.content.clone(),
            })) {
                Ok(payload) => payload,
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to convert chunk to payload: {}", e));
                }
            };

            let point_id = generate_point_id(
                &file_path_relative,
                chunk.chunk.start_line,
                chunk.chunk.end_line,
                &chunk.chunk.symbol_name,
            );

            Ok(PointStruct::new(point_id, chunk.embedding, payload))
        })
        .collect::<Result<Vec<_>, anyhow::Error>>();

    let points = match points {
        Ok(points) => points,
        Err(e) => {
            cleanup_collection(&collection_id_for_cleanup, &e.to_string()).await;
            return Err(e);
        }
    };

    // Save the chunks to the vector db
    if let Err(e) = QDRANT_CLIENT
        .upsert_points(UpsertPointsBuilder::new(collection_id.clone(), points))
        .await
    {
        let error_msg = format!("Failed to upsert points to collection {collection_id}: {e}");
        cleanup_collection(&collection_id_for_cleanup, &error_msg).await;
        return Err(anyhow::anyhow!(error_msg));
    }

    // Save the state file - this should be done before changing directory
    let file_states = match collect_supported_file_states(root_path.as_ref()) {
        Ok(states) => states,
        Err(e) => {
            let error_msg = format!("Failed to collect file states: {e}");
            cleanup_collection(&collection_id_for_cleanup, &error_msg).await;
            return Err(anyhow::anyhow!(error_msg));
        }
    };

    // Change to the target directory
    if let Err(e) = std::env::set_current_dir(root_path.as_ref()) {
        let error_msg = format!(
            "Failed to change directory to {}: {e}",
            root_path.as_ref().display()
        );
        cleanup_collection(&collection_id_for_cleanup, &error_msg).await;
        return Err(anyhow::anyhow!(error_msg));
    }

    let state = CodebaseState { file_states };
    if let Err(e) = state.to_file(None) {
        let error_msg = format!("Failed to save state file: {e}");
        cleanup_collection(&collection_id_for_cleanup, &error_msg).await;
        return Err(anyhow::anyhow!(error_msg));
    }

    info!("Successfully initialized session with collection: {collection_id}");
    Ok(())
}

/// restore_vector_db checks for vector updates when reopening the project
/// it will compare the content hash of the file with the last modified time
/// if the content hash is different, it will update the vector db
/// if the content hash is the same, it will skip the update
pub async fn restore_session<P: AsRef<Path>>(root_path: P) -> Result<(), anyhow::Error> {
    let index_file_path = root_path.as_ref().join(".rua.index.json");
    info!("looking for index file at {}", index_file_path.display());

    match fs::exists(&index_file_path) {
        Ok(true) => {
            info!("Found existing index file, checking for changes...");

            // 1. Load the existing state
            std::env::set_current_dir(root_path.as_ref())?;
            let saved_state = CodebaseState::from_file(None)?;

            // 2. Discover current files and build current state
            let current_file_states = collect_supported_file_states(root_path.as_ref())?;
            let seen_files: HashSet<String> = current_file_states.keys().cloned().collect();

            // 3. Compare states and categorize files
            let mut added_files = Vec::new();
            let mut modified_files = Vec::new();
            let mut deleted_files = Vec::new();

            // Find added and modified files
            for (file_path, current_state) in &current_file_states {
                match saved_state.file_states.get(file_path) {
                    Some(saved_state) => {
                        // File existed before, check if modified
                        if current_state.content_md5 != saved_state.content_md5 {
                            debug!("File modified: {file_path}");
                            modified_files.push(file_path.clone());
                        }
                    }
                    None => {
                        // New file
                        debug!("File added: {file_path}");
                        added_files.push(file_path.clone());
                    }
                }
            }

            // Find deleted files
            for file_path in saved_state.file_states.keys() {
                if !seen_files.contains(file_path) {
                    debug!("File deleted: {file_path}");
                    deleted_files.push(file_path.clone());
                }
            }

            info!(
                "Changes detected - Added: {}, Modified: {}, Deleted: {}",
                added_files.len(),
                modified_files.len(),
                deleted_files.len()
            );
            info!(
                "Using collection: {}",
                generate_collection_id(root_path.as_ref()).as_str()
            );

            // 4. Update vector database if there are changes
            if !added_files.is_empty() || !modified_files.is_empty() || !deleted_files.is_empty() {
                // Handle file deletions - remove points for deleted and modified files
                let files_to_delete: Vec<String> = deleted_files
                    .iter()
                    .chain(modified_files.iter())
                    .cloned()
                    .collect();

                if !files_to_delete.is_empty() {
                    debug!(
                        "Removing points for {} files (deleted: {}, modified: {})",
                        files_to_delete.len(),
                        deleted_files.len(),
                        modified_files.len()
                    );

                    // Create filter to match points with any of the file paths to delete
                    let conditions: Vec<Condition> = files_to_delete
                        .iter()
                        .map(|file_path| Condition::matches("file_path", file_path.clone()))
                        .collect();

                    let filter = Filter::should(conditions);

                    // Delete all points matching this filter in a single operation
                    QDRANT_CLIENT
                        .delete_points(
                            DeletePointsBuilder::new(
                                generate_collection_id(root_path.as_ref()).as_str(),
                            )
                            .points(filter),
                        )
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Failed to delete points for {} files: {}",
                                files_to_delete.len(),
                                e
                            )
                        })?;
                    info!(
                        "Deleted points for {} files (deleted: {}, modified: {})",
                        files_to_delete.len(),
                        deleted_files.len(),
                        modified_files.len()
                    );
                }

                // Process added and modified files - chunk and insert new content
                let files_to_process: Vec<String> = added_files
                    .iter()
                    .chain(modified_files.iter())
                    .cloned()
                    .collect();

                if !files_to_process.is_empty() {
                    info!(
                        "Processing {} files for insertion (added: {}, modified: {})",
                        files_to_process.len(),
                        added_files.len(),
                        modified_files.len()
                    );

                    let opts = ChunkingOptions::default();
                    let mut all_chunks = Vec::new();

                    // Process each file individually
                    for file_path in &files_to_process {
                        let full_file_path = root_path.as_ref().join(file_path);

                        match chunk_codefile(&full_file_path, opts.clone()).await {
                            Ok(mut chunks) => {
                                debug!("Generated {} chunks for file: {}", chunks.len(), file_path);
                                all_chunks.append(&mut chunks);
                            }
                            Err(e) => {
                                warn!("Failed to chunk file {}: {}", file_path, e);
                                continue;
                            }
                        }
                    }

                    info!(
                        "Generated {} chunks for {} files",
                        all_chunks.len(),
                        files_to_process.len()
                    );

                    if !all_chunks.is_empty() {
                        // Convert chunks to points with metadata
                        let mut points = Vec::new();
                        for chunk in all_chunks {
                            let file_path_relative = chunk
                                .chunk
                                .file_path
                                .strip_prefix(root_path.as_ref())
                                .unwrap_or(&chunk.chunk.file_path)
                                .to_string_lossy()
                                .to_string();

                            let payload = match Payload::try_from(json!({
                                "file_path": file_path_relative.clone(),
                                "start_line": chunk.chunk.start_line,
                                "end_line": chunk.chunk.end_line,
                                "symbol_name": chunk.chunk.symbol_name.clone(),
                                "symbol_kind": chunk.chunk.symbol_kind.clone(),
                                "is_container": chunk.chunk.chunk_metadata.is_container,
                                "original_size_lines": chunk.chunk.chunk_metadata.original_size_lines,
                                "is_split": chunk.chunk.chunk_metadata.is_split,
                                "chunk_depth": chunk.chunk.chunk_metadata.chunk_depth,
                                "context": chunk.chunk.context.clone(),
                                "content": chunk.chunk.content.clone(),
                            })) {
                                Ok(payload) => payload,
                                Err(e) => {
                                    warn!(
                                        "Failed to convert chunk to payload for file {}: {}",
                                        file_path_relative, e
                                    );
                                    continue;
                                }
                            };

                            let point_id = generate_point_id(
                                &file_path_relative,
                                chunk.chunk.start_line,
                                chunk.chunk.end_line,
                                &chunk.chunk.symbol_name,
                            );

                            points.push(PointStruct::new(point_id, chunk.embedding, payload));
                        }

                        // Upsert points (this will automatically update existing points with same ID)
                        QDRANT_CLIENT
                            .upsert_points(UpsertPointsBuilder::new(
                                generate_collection_id(root_path.as_ref()).as_str(),
                                points,
                            ))
                            .await?;

                        info!(
                            "Successfully inserted points for {} files (added: {}, modified: {})",
                            files_to_process.len(),
                            added_files.len(),
                            modified_files.len()
                        );
                    }
                }

                // 5. Save the updated state file
                let new_state = CodebaseState {
                    file_states: current_file_states,
                };
                new_state.to_file(None)?;
                info!("Updated state file with current file states");
            } else {
                info!("No changes detected, vector database is up to date");
            }
        }
        Ok(false) => {
            info!("No existing index file found, initializing new session...");
            init_session(root_path).await?;
        }
        Err(e) => {
            return Err(anyhow::anyhow!(
                "Failed to check if state file exists: {}",
                e
            ));
        }
    }
    Ok(())
}
