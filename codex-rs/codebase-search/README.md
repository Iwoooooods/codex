# Codebase Search

This module provides semantic search capabilities for codebases using vector embeddings and Qdrant as the vector database.

## Features

- **Hierarchical code chunking**: Intelligently splits code into meaningful chunks based on symbols and structure
- **Vector embeddings**: Converts code chunks into high-dimensional vectors for semantic search
- **Global embedding client**: Lazy-loaded, configurable client for different embedding providers
- **Qdrant integration**: Stores and searches vectors using Qdrant vector database
- **Multi-language support**: Supports Rust, Python, and Go codebases

## Configuration

### Embedding Provider Configuration

The embedding client can be configured using environment variables:

- `CODEX_EMBEDDING_PROVIDER`: Provider name (`siliconflow`, `openai`, `cohere`)
- `CODEX_EMBEDDING_API_URL`: Custom API endpoint URL
- `CODEX_EMBEDDING_API_KEY`: API key for authentication
- `CODEX_EMBEDDING_MODEL`: Model name to use
- `CODEX_EMBEDDING_BATCH_SIZE`: Batch size for embedding requests (default: 10)
- `CODEX_EMBEDDING_TIMEOUT`: Request timeout in seconds (default: 30)

#### Provider Defaults

**SiliconFlow** (default):
- API URL: `https://api.siliconflow.cn/v1/embeddings`
- Model: `Qwen/Qwen3-Embedding-8B`

**OpenAI**:
- API URL: `https://api.openai.com/v1/embeddings`
- Model: `text-embedding-3-large`

**Cohere**:
- API URL: `https://api.cohere.ai/v1/embed`
- Model: `embed-english-v3.0`

### Qdrant Configuration

By default, the system connects to Qdrant at `http://localhost:6334`.

## Usage

### Initializing a Session

```rust
use codebase_search::vector_db::{init_session, restore_session};

// For a new project
init_session("/path/to/codebase").await?;

// For an existing project (will detect changes and update incrementally)
restore_session("/path/to/codebase").await?;
```

### Searching

```rust
use codebase_search::retriever::search_codebase;

search_codebase("authentication flow".to_string()).await?;
```

## Architecture

The system uses a global, lazy-loaded embedding client that is configured once and reused throughout the application. This ensures consistent configuration and efficient resource usage.

- **Symbol Parsing**: Extracts semantic symbols from code files
- **Hierarchical Chunking**: Creates meaningful code chunks respecting symbol boundaries
- **Embedding**: Converts chunks to vectors using configurable providers
- **Vector Storage**: Stores embeddings in Qdrant with metadata
- **Incremental Updates**: Only re-processes changed files on subsequent runs

## Error Handling

The system follows Rust best practices:
- Uses `anyhow::Error` for error handling
- Avoids `unwrap()` in favor of proper error propagation
- Provides meaningful error messages for debugging 