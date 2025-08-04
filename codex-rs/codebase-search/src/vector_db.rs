use std::sync::Arc;
use std::sync::LazyLock;

use serde_json::json;

use crate::chunker::ChunkingOptions;
use crate::chunker::index_codebase;
use crate::embedding::QDRANT_EMBEDDING_DIMENSION;
use qdrant_client::Payload;
use qdrant_client::Qdrant;
use qdrant_client::qdrant::CreateCollectionBuilder;
use qdrant_client::qdrant::Distance;
use qdrant_client::qdrant::PointStruct;
use qdrant_client::qdrant::UpsertPointsBuilder;
use qdrant_client::qdrant::VectorParamsBuilder;
use sha2::Digest;
use sha2::Sha256;
use uuid::Uuid;

static QDRANT_CLIENT: LazyLock<Arc<Qdrant>> =
    LazyLock::new(|| match Qdrant::from_url("http://localhost:6334").build() {
        Ok(client) => Arc::new(client),
        Err(e) => panic!("Failed to create Qdrant client: {e}"),
    });

/// Generate a unique collection ID from a root path using SHA-256 hashing
/// This creates a deterministic, unique identifier that's safe for use as a collection name
fn generate_collection_id(root_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(root_path.as_bytes());
    let hash = hasher.finalize();
    let hash_str = format!("{hash:x}");

    // Take the first 16 characters of the hash to keep it reasonably short
    // while still maintaining uniqueness
    format!("codex_{}", &hash_str[..16])
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
pub async fn init_vector_db(root_path: &str) -> Result<(), anyhow::Error> {
    // 1. generate a unique collection id based on the root path
    let collection_id = generate_collection_id(root_path);

    // 2. create a new collection
    QDRANT_CLIENT
        .create_collection(CreateCollectionBuilder::new(&collection_id).vectors_config(
            VectorParamsBuilder::new(QDRANT_EMBEDDING_DIMENSION as u64, Distance::Cosine),
        ))
        .await?;
    // 3. index the project
    let opts = ChunkingOptions::default();
    let chunks = index_codebase(root_path, opts).await?;
    // chunks to points with metadata
    let points = chunks
        .into_iter()
        .map(|chunk| {
            let payload = match Payload::try_from(json!({
                "file_path": chunk.chunk.file_path.clone(),
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
                Err(e) => panic!("Failed to convert chunk to payload: {e}"),
            };
            PointStruct::new(Uuid::new_v4().to_string(), chunk.embedding, payload)
        })
        .collect::<Vec<_>>();
    // 4. save the chunks to the vector db
    QDRANT_CLIENT
        .upsert_points(UpsertPointsBuilder::new(&collection_id, points))
        .await?;
    // 5. save the state file

    Ok(())
}

// /// restore_vector_db checks for vector updates when reopening the project
// /// it will compare the content hash of the file with the last modified time
// /// if the content hash is different, it will update the vector db
// /// if the content hash is the same, it will skip the update
// pub fn restore_vector_db(root_path: &str) -> Result<(), anyhow::Error> {
//     // 1. get the collection id
//     let collection_id = QDRANT_CLIENT.get_collection(root_path).await?;
//     // 2. get the points
//     let points = QDRANT_CLIENT
//         .get_points(collection_id, &[PointId::new(0)])
//         .await?;
//     // 3. update the points
//     Ok(())
// }
