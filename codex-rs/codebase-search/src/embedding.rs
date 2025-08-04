use crate::chunker::CodeChunk;
use anyhow::Result;
use anyhow::anyhow;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::str::FromStr;
use tracing::error;
use tracing::info;

pub const QDRANT_EMBEDDING_MODEL: &str = "Qwen/Qwen3-Embedding-8B";
pub const QDRANT_EMBEDDING_DIMENSION: usize = 4096;

/// Configuration for embedding model providers
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    /// The model provider (e.g., "siliconflow", "openai", "cohere")
    pub provider: String,
    /// The API endpoint URL
    pub api_url: String,
    /// The API key/token for authentication
    pub api_key: String,
    /// The specific model to use
    pub model: String,
    /// Maximum batch size for embedding requests
    pub batch_size: usize,
    /// Request timeout in seconds
    pub timeout_seconds: u64,
    /// Additional headers to include in requests
    pub additional_headers: HashMap<String, String>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: "siliconflow".to_string(),
            api_url: "https://api.siliconflow.cn/v1/embeddings".to_string(),
            api_key: String::from_str("sk-xzmxwlbvzdbgsgejawqjamccrifkwjabcpmmgenprsfudnpt")
                .unwrap_or_default(),
            model: "Qwen/Qwen3-Embedding-8B".to_string(),
            batch_size: 10,
            timeout_seconds: 30,
            additional_headers: HashMap::new(),
        }
    }
}

/// Response structure for embedding API calls
#[derive(Debug, Deserialize)]
pub struct EmbeddingResponse {
    pub data: Vec<EmbeddingData>,
    pub model: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
pub struct EmbeddingData {
    pub embedding: Vec<f32>,
    pub index: usize,
    pub object: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Usage {
    pub prompt_tokens: Option<usize>,
    pub total_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
}

/// Request structure for embedding API calls
#[derive(Debug, Serialize)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: Vec<String>,
}

/// Represents an embedded code chunk with its vector representation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddedChunk {
    /// The original code chunk
    pub chunk: CodeChunk,
    /// The embedding vector
    pub embedding: Vec<f32>,
    /// The model used for embedding
    pub model: String,
    /// Timestamp when the embedding was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Main embedding client that handles communication with embedding providers
pub struct EmbeddingClient {
    config: EmbeddingConfig,
    client: Client,
}

impl EmbeddingClient {
    /// Create a new embedding client with the given configuration
    pub fn new(config: EmbeddingConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_seconds))
            .build()?;

        Ok(Self { config, client })
    }

    /// Embed a single code chunk
    pub async fn embed_chunk(&self, chunk: &CodeChunk) -> Result<EmbeddedChunk> {
        let embeddings = self.embed_texts(&[chunk.content.clone()]).await?;

        if embeddings.is_empty() {
            return Err(anyhow!("No embeddings returned for chunk"));
        }

        Ok(EmbeddedChunk {
            chunk: chunk.clone(),
            embedding: embeddings[0].clone(),
            model: self.config.model.clone(),
            created_at: chrono::Utc::now(),
        })
    }

    /// Embed multiple code chunks in batches
    pub async fn embed_chunks(&self, chunks: &[CodeChunk]) -> Result<Vec<EmbeddedChunk>> {
        if chunks.is_empty() {
            return Ok(vec![]);
        }

        info!(
            "Embedding {} chunks using {}",
            chunks.len(),
            self.config.provider
        );

        let mut embedded_chunks = Vec::new();
        let mut current_batch = Vec::new();
        let mut batch_texts = Vec::new();

        for (i, chunk) in chunks.iter().enumerate() {
            current_batch.push(chunk);
            batch_texts.push(chunk.content.clone());

            // Process batch when it reaches the size limit or at the end
            if batch_texts.len() >= self.config.batch_size || i == chunks.len() - 1 {
                let embeddings = self.embed_texts(&batch_texts).await?;

                if embeddings.len() != current_batch.len() {
                    return Err(anyhow!(
                        "Embedding count mismatch: expected {}, got {}",
                        current_batch.len(),
                        embeddings.len()
                    ));
                }

                for (chunk, embedding) in current_batch.iter().zip(embeddings.iter()) {
                    embedded_chunks.push(EmbeddedChunk {
                        chunk: (*chunk).clone(),
                        embedding: embedding.clone(),
                        model: self.config.model.clone(),
                        created_at: chrono::Utc::now(),
                    });
                }

                // Reset for next batch
                current_batch.clear();
                batch_texts.clear();
            }
        }

        info!("Successfully embedded {} chunks", embedded_chunks.len());
        Ok(embedded_chunks)
    }

    /// Send embedding request to the configured provider
    async fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let request = EmbeddingRequest {
            model: self.config.model.clone(),
            input: texts.to_vec(),
        };

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {}", self.config.api_key).parse()?,
        );
        headers.insert("Content-Type", "application/json".parse()?);

        // Add additional headers
        for (key, value) in &self.config.additional_headers {
            headers.insert(
                key.parse::<reqwest::header::HeaderName>()?,
                value.parse::<reqwest::header::HeaderValue>()?,
            );
        }

        let response = self
            .client
            .post(&self.config.api_url)
            .headers(headers)
            .json(&request)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            error!("Embedding API error: {}", error_text);
            return Err(anyhow!(
                "Embedding API request failed with status: {}",
                status
            ));
        }

        let embedding_response: EmbeddingResponse = response.json().await?;
        // Sort embeddings by index to maintain order
        let mut embeddings: Vec<_> = embedding_response.data.into_iter().collect();
        embeddings.sort_by_key(|data| data.index);

        Ok(embeddings.into_iter().map(|data| data.embedding).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::ChunkMetadata;
    use std::path::PathBuf;

    // Note: These tests would require a real API key to work
    #[tokio::test]
    async fn test_embed_chunk_siliconflow() {
        let _ = tracing_subscriber::fmt::try_init();

        let config = EmbeddingConfig::default();
        let client = match EmbeddingClient::new(config) {
            Ok(client) => client,
            Err(e) => panic!("Failed to create embedding client: {e}"),
        };
        let chunk = CodeChunk {
            content: "fn test_function() {\n    println!(\"Hello, world!\");\n}".to_string(),
            file_path: PathBuf::from("test.rs"),
            start_line: 1,
            end_line: 3,
            symbol_name: "test_function".to_string(),
            symbol_kind: "Function".to_string(),
            context: Some("mod test".to_string()),
            chunk_metadata: ChunkMetadata {
                is_split: false,
                original_size_lines: 3,
                chunk_depth: 0,
                is_container: false,
            },
        };
        let result = match client.embed_chunk(&chunk).await {
            Ok(result) => result,
            Err(e) => panic!("Failed to embed chunk: {e}"),
        };

        assert!(!result.embedding.is_empty());
        assert_eq!(result.chunk.symbol_name, "test_function");

        info!("Embedded chunk: {:?}", result);
    }
}
