# ToolSchema Macro

The `ToolSchema` derive macro automatically implements the `ToJsonSchema` trait for structs, parsing struct definitions into JSON schema format for OpenAI tool calls.

## Overview

This macro generates JSON schema from Rust struct definitions at compile time, ensuring type safety and automatic synchronization between your data structures and their corresponding OpenAI tool schemas.

## Basic Usage

### 1. Define Your Parameter Struct

```rust
use macros::ToolSchema;
use serde::{Deserialize, Serialize};

#[derive(ToolSchema, Deserialize, Serialize)]
pub struct WeatherParams {
    pub city: String,
    pub temperature: Option<f64>,
    pub include_forecast: bool,
}
```

### 2. Use with Generic Helper Functions

```rust
use crate::openai_tools::{create_tool_from_struct, ToJsonSchema};

// The macro automatically implements ToJsonSchema for WeatherParams
let weather_tool = create_tool_from_struct::<WeatherParams>(
    "get_weather",
    "Get current weather information for a city"
);
```

## Supported Rust Types

The macro automatically maps Rust types to JSON schema types:

| Rust Type | JSON Schema Type | Notes |
|-----------|------------------|-------|
| `String` | `string` | Text values |
| `&str` | `string` | String references |
| `i32`, `i64`, `u32`, `u64` | `number` | Integer types |
| `f32`, `f64` | `number` | Floating point types |
| `bool` | `boolean` | Boolean values |
| `Vec<T>` | `array` | Arrays with items of type T |
| `Option<T>` | `T` | Optional fields (not required) |
| Other types | `string` | Default fallback |

## Debugging Generated Code

To see what the macro generates, you can use `cargo expand`:

```bash
# Install cargo-expand if you haven't
cargo install cargo-expand

# Expand the macro to see generated code
cargo expand --package codex-core
```
