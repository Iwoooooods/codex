use qdrant_client::qdrant::SearchParamsBuilder;
use qdrant_client::qdrant::SearchPointsBuilder;
use qdrant_client::qdrant::Value as QdrantValue;
use tracing::info;

use crate::chunker::ChunkMetadata;
use crate::chunker::CodeChunk;
use crate::vector_db::QDRANT_CLIENT;
use crate::vector_db::generate_collection_id;
use std::path::Path;
use std::path::PathBuf;

/// A search result containing the code chunk and its similarity score
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub chunk: CodeChunk,
    pub score: f32,
}

/// Search codebase with a query and return structured results
pub async fn search_codebase<P: AsRef<Path>>(
    query: String,
    root_path: P,
    limit: usize,
    min_score: f32,
) -> Result<Vec<SearchResult>, anyhow::Error> {
    let collection_id = generate_collection_id(root_path.as_ref());
    info!("Searching collection: {}", collection_id);

    // Embed the query text using global embedding client
    let embedding_client = crate::embedding::get_embedding_client()?;
    let query_vector = embedding_client.embed_query(&query).await?;
    info!(
        "Embedded query '{}' into vector of dimension {}",
        query,
        query_vector.len()
    );

    // Perform vector search using the embedded query
    let search_response = QDRANT_CLIENT
        .clone()
        .search_points(
            SearchPointsBuilder::new(collection_id.as_str(), query_vector, limit as u64)
                .with_payload(true)
                .params(SearchParamsBuilder::default()),
        )
        .await?;

    info!("Found {} search results", search_response.result.len());

    // Convert Qdrant results to our SearchResult structure
    let mut results = Vec::new();

    for scored_point in search_response.result {
        let score = scored_point.score;

        // Skip results below minimum score threshold
        if score < min_score {
            continue;
        }

        let payload = scored_point.payload;

        // Extract fields from payload with proper error handling
        let file_path = extract_string_field(&payload, "file_path")?;
        let start_line = extract_u64_field(&payload, "start_line")? as usize;
        let end_line = extract_u64_field(&payload, "end_line")? as usize;
        let symbol_name = extract_string_field(&payload, "symbol_name")?;
        let symbol_kind = extract_string_field(&payload, "symbol_kind")?;
        let content = extract_string_field(&payload, "content")?;

        // Optional fields
        let context = extract_optional_string_field(&payload, "context");

        // Extract chunk metadata
        let is_container = extract_optional_bool_field(&payload, "is_container").unwrap_or(false);
        let original_size_lines = extract_optional_u64_field(&payload, "original_size_lines")
            .map(|v| v as usize)
            .unwrap_or(end_line - start_line + 1);
        let is_split = extract_optional_bool_field(&payload, "is_split").unwrap_or(false);
        let chunk_depth = extract_optional_u64_field(&payload, "chunk_depth")
            .map(|v| v as usize)
            .unwrap_or(0);

        let chunk_metadata = ChunkMetadata {
            is_container,
            original_size_lines,
            is_split,
            chunk_depth,
        };

        let chunk = CodeChunk {
            content,
            file_path: PathBuf::from(file_path),
            start_line,
            end_line,
            symbol_name,
            symbol_kind,
            context,
            chunk_metadata,
        };

        results.push(SearchResult { chunk, score });
    }

    // Sort by score descending
    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(results)
}

/// Helper function to extract string field from Qdrant payload
fn extract_string_field(
    payload: &std::collections::HashMap<String, QdrantValue>,
    field: &str,
) -> Result<String, anyhow::Error> {
    payload
        .get(field)
        .and_then(|v| match v {
            QdrantValue {
                kind: Some(qdrant_client::qdrant::value::Kind::StringValue(s)),
            } => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid '{}' field in payload", field))
}

/// Helper function to extract u64 field from Qdrant payload
fn extract_u64_field(
    payload: &std::collections::HashMap<String, QdrantValue>,
    field: &str,
) -> Result<u64, anyhow::Error> {
    payload
        .get(field)
        .and_then(|v| match v {
            QdrantValue {
                kind: Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)),
            } => Some(*i as u64),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("Missing or invalid '{}' field in payload", field))
}

/// Helper function to extract optional string field from Qdrant payload
fn extract_optional_string_field(
    payload: &std::collections::HashMap<String, QdrantValue>,
    field: &str,
) -> Option<String> {
    payload.get(field).and_then(|v| match v {
        QdrantValue {
            kind: Some(qdrant_client::qdrant::value::Kind::StringValue(s)),
        } => Some(s.clone()),
        _ => None,
    })
}

/// Helper function to extract optional u64 field from Qdrant payload
fn extract_optional_u64_field(
    payload: &std::collections::HashMap<String, QdrantValue>,
    field: &str,
) -> Option<u64> {
    payload.get(field).and_then(|v| match v {
        QdrantValue {
            kind: Some(qdrant_client::qdrant::value::Kind::IntegerValue(i)),
        } => Some(*i as u64),
        _ => None,
    })
}

/// Helper function to extract optional bool field from Qdrant payload
fn extract_optional_bool_field(
    payload: &std::collections::HashMap<String, QdrantValue>,
    field: &str,
) -> Option<bool> {
    payload.get(field).and_then(|v| match v {
        QdrantValue {
            kind: Some(qdrant_client::qdrant::value::Kind::BoolValue(b)),
        } => Some(*b),
        _ => None,
    })
}
