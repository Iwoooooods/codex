use crate::chunker::CodeChunk;
use anyhow::Result;
use anyhow::anyhow;
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::LazyLock;
use tracing::error;
use tracing::info;

pub const QDRANT_EMBEDDING_MODEL: &str = "Qwen/Qwen3-Embedding-8B";
pub const QDRANT_EMBEDDING_DIMENSION: usize = 4096;

/// Lazy-loaded global embedding client for interacting with embedding providers
/// This client is configured based on environment variables or defaults to SiliconFlow
pub(crate) static EMBEDDING_CLIENT: LazyLock<Result<Arc<EmbeddingClient>, anyhow::Error>> =
    LazyLock::new(|| {
        let config = create_embedding_config();
        EmbeddingClient::new(config)
            .map(Arc::new)
            .map_err(|e| anyhow::anyhow!("Failed to create embedding client: {e}"))
    });

/// Get the global embedding client, returning an error if initialization failed
pub(crate) fn get_embedding_client() -> Result<Arc<EmbeddingClient>, anyhow::Error> {
    match &*EMBEDDING_CLIENT {
        Ok(client) => Ok(Arc::clone(client)),
        Err(e) => Err(anyhow::anyhow!(
            "Embedding client initialization failed: {e}"
        )),
    }
}

/// Create embedding configuration from environment variables or defaults
fn create_embedding_config() -> EmbeddingConfig {
    let provider =
        std::env::var("CODEX_EMBEDDING_PROVIDER").unwrap_or_else(|_| "siliconflow".to_string());

    let (api_url, model) = match provider.as_str() {
        "openai" => (
            std::env::var("CODEX_EMBEDDING_API_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1/embeddings".to_string()),
            std::env::var("CODEX_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-large".to_string()),
        ),
        "cohere" => (
            std::env::var("CODEX_EMBEDDING_API_URL")
                .unwrap_or_else(|_| "https://api.cohere.ai/v1/embed".to_string()),
            std::env::var("CODEX_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "embed-english-v3.0".to_string()),
        ),
        "siliconflow" | _ => (
            std::env::var("CODEX_EMBEDDING_API_URL")
                .unwrap_or_else(|_| "https://api.siliconflow.cn/v1/embeddings".to_string()),
            std::env::var("CODEX_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "Qwen/Qwen3-Embedding-8B".to_string()),
        ),
    };

    let api_key = std::env::var("CODEX_EMBEDDING_API_KEY")
        .or_else(|_| std::env::var("OPENAI_API_KEY")) // Fallback for OpenAI
        .unwrap_or_else(|_| {
            // Default API key for SiliconFlow (from existing default config)
            "sk-xzmxwlbvzdbgsgejawqjamccrifkwjabcpmmgenprsfudnpt".to_string()
        });

    let batch_size = std::env::var("CODEX_EMBEDDING_BATCH_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let timeout_seconds = std::env::var("CODEX_EMBEDDING_TIMEOUT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(30);

    EmbeddingConfig {
        provider,
        api_url,
        api_key,
        model,
        batch_size,
        timeout_seconds,
        additional_headers: HashMap::new(),
    }
}

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

    /// Embed a query string for similarity search
    pub async fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        let embeddings = self.embed_texts(&[query.to_string()]).await?;

        if embeddings.is_empty() {
            return Err(anyhow!("No embeddings returned for query"));
        }

        Ok(embeddings[0].clone())
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
                "Embedding API request failed with status: {}, with payload: {:?}",
                status,
                request.input,
            ));
        }

        let embedding_response: EmbeddingResponse = response.json().await?;
        // Sort embeddings by index to maintain order
        let mut embeddings: Vec<_> = embedding_response.data.into_iter().collect();
        embeddings.sort_by_key(|data| data.index);

        Ok(embeddings.into_iter().map(|data| data.embedding).collect())
    }
}
