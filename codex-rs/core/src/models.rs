use std::collections::HashMap;

use crate::exec_env::create_env;
use base64::Engine;
use mcp_types::CallToolResult;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::ser::Serializer;

use crate::codex::Session;
use crate::exec::ExecParams;
use crate::openai_tools::JsonSchema;
use crate::openai_tools::ToJsonSchema;
use crate::protocol::InputItem;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseInputItem {
    Message {
        role: String,
        content: Vec<ContentItem>,
    },
    FunctionCallOutput {
        call_id: String,
        output: FunctionCallOutputPayload,
    },
    McpToolCallOutput {
        call_id: String,
        result: Result<CallToolResult, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentItem {
    InputText { text: String },
    InputImage { image_url: String },
    OutputText { text: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ResponseItem {
    Message {
        id: Option<String>,
        role: String,
        content: Vec<ContentItem>,
    },
    Reasoning {
        id: String,
        summary: Vec<ReasoningItemReasoningSummary>,
        encrypted_content: Option<String>,
    },
    LocalShellCall {
        /// Set when using the chat completions API.
        id: Option<String>,
        /// Set when using the Responses API.
        call_id: Option<String>,
        status: LocalShellStatus,
        action: LocalShellAction,
    },
    FunctionCall {
        id: Option<String>,
        name: String,
        // The Responses API returns the function call arguments as a *string* that contains
        // JSON, not as an already‑parsed object. We keep it as a raw string here and let
        // Session::handle_function_call parse it into a Value. This exactly matches the
        // Chat Completions + Responses API behavior.
        arguments: String,
        call_id: String,
    },
    // NOTE: The input schema for `function_call_output` objects that clients send to the
    // OpenAI /v1/responses endpoint is NOT the same shape as the objects the server returns on the
    // SSE stream. When *sending* we must wrap the string output inside an object that includes a
    // required `success` boolean. The upstream TypeScript CLI does this implicitly. To ensure we
    // serialize exactly the expected shape we introduce a dedicated payload struct and flatten it
    // here.
    FunctionCallOutput {
        call_id: String,
        output: FunctionCallOutputPayload,
    },
    #[serde(other)]
    Other,
}

impl From<ResponseInputItem> for ResponseItem {
    fn from(item: ResponseInputItem) -> Self {
        match item {
            ResponseInputItem::Message { role, content } => Self::Message {
                role,
                content,
                id: None,
            },
            ResponseInputItem::FunctionCallOutput { call_id, output } => {
                Self::FunctionCallOutput { call_id, output }
            }
            ResponseInputItem::McpToolCallOutput { call_id, result } => Self::FunctionCallOutput {
                call_id,
                output: FunctionCallOutputPayload {
                    success: Some(result.is_ok()),
                    content: result.map_or_else(
                        |tool_call_err| format!("err: {tool_call_err:?}"),
                        |result| {
                            serde_json::to_string(&result)
                                .unwrap_or_else(|e| format!("JSON serialization error: {e}"))
                        },
                    ),
                },
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalShellStatus {
    Completed,
    InProgress,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LocalShellAction {
    Exec(LocalShellExecAction),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalShellExecAction {
    pub command: Vec<String>,
    pub timeout_ms: Option<u64>,
    pub working_directory: Option<String>,
    pub env: Option<HashMap<String, String>>,
    pub user: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReasoningItemReasoningSummary {
    SummaryText { text: String },
}

impl From<Vec<InputItem>> for ResponseInputItem {
    fn from(items: Vec<InputItem>) -> Self {
        Self::Message {
            role: "user".to_string(),
            content: items
                .into_iter()
                .filter_map(|c| match c {
                    InputItem::Text { text } => Some(ContentItem::InputText { text }),
                    InputItem::Image { image_url } => Some(ContentItem::InputImage { image_url }),
                    InputItem::LocalImage { path } => match std::fs::read(&path) {
                        Ok(bytes) => {
                            let mime = mime_guess::from_path(&path)
                                .first()
                                .map(|m| m.essence_str().to_owned())
                                .unwrap_or_else(|| "application/octet-stream".to_string());
                            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                            Some(ContentItem::InputImage {
                                image_url: format!("data:{mime};base64,{encoded}"),
                            })
                        }
                        Err(err) => {
                            tracing::warn!(
                                "Skipping image {} – could not read file: {}",
                                path.display(),
                                err
                            );
                            None
                        }
                    },
                })
                .collect::<Vec<ContentItem>>(),
        }
    }
}

/// If the `name` of a `ResponseItem::FunctionCall` is either `container.exec`
/// or shell`, the `arguments` field should deserialize to this struct.
#[derive(macros::ToolSchema, Deserialize, Debug, Clone, PartialEq)]
pub struct ShellToolCallParams {
    pub command: Vec<String>,
    pub workdir: Option<String>,

    /// This is the maximum time in milliseconds that the command is allowed to run.
    #[serde(rename = "timeout")]
    pub timeout: Option<u64>,
}

impl ShellToolCallParams {
    pub(crate) fn to_exec_params(&self, sess: &Session) -> ExecParams {
        ExecParams {
            command: self.command.clone(),
            cwd: sess.resolve_path(self.workdir.clone()),
            timeout_ms: self.timeout,
            env: create_env(&sess.shell_environment_policy),
        }
    }
}

#[derive(macros::ToolSchema, Deserialize, Debug, Clone, PartialEq)]
pub struct ReadFileToolCallParams {
    pub path: String,
    pub should_read_entire_file: bool,
    pub start_line_one_indexed: Option<u64>,
    pub end_line_one_indexed_inclusive: Option<u64>,
    pub explanation: Option<String>,
}

impl ReadFileToolCallParams {
    pub(crate) fn to_exec_params(&self, sess: &Session) -> ExecParams {
        let command = if self.should_read_entire_file {
            // use `cat` to read the entire file
            vec!["cat".to_string(), self.path.clone()]
        } else {
            // use `sed` to read specific lines of a file
            let start_line = self.start_line_one_indexed.unwrap_or(1);
            let end_line = self.end_line_one_indexed_inclusive.unwrap_or(1);
            vec![
                "sed".to_string(),
                "-n".to_string(),
                format!("{start_line},{end_line}p"),
                self.path.clone(),
            ]
        };
        ExecParams {
            command,
            cwd: sess.resolve_path(None),
            timeout_ms: None,
            env: create_env(&sess.shell_environment_policy),
        }
    }
    /// Validates the parameters to ensure logical consistency
    pub fn validate(&self) -> Result<(), String> {
        // Validate line numbers when both are present
        if let (Some(start_line), Some(end_line)) = (
            self.start_line_one_indexed,
            self.end_line_one_indexed_inclusive,
        ) {
            if start_line > end_line {
                return Err(format!(
                    "start_line_one_indexed ({start_line}) must be less than or equal to end_line_one_indexed_inclusive ({end_line})"
                ));
            }

            // Validate that line numbers are valid (greater than 0)
            if start_line == 0 {
                return Err(
                    "start_line_one_indexed must be greater than 0 (one-indexed)".to_string(),
                );
            }
            if end_line == 0 {
                return Err(
                    "end_line_one_indexed_inclusive must be greater than 0 (one-indexed)"
                        .to_string(),
                );
            }
        }

        // Validate that we have line numbers when not reading entire file
        if !self.should_read_entire_file
            && (self.start_line_one_indexed.is_none()
                || self.end_line_one_indexed_inclusive.is_none())
        {
            return Err("start_line_one_indexed and end_line_one_indexed_inclusive are required when should_read_entire_file is false".to_string());
        }

        // Validate path is not empty
        if self.path.trim().is_empty() {
            return Err("path cannot be empty".to_string());
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct FunctionCallOutputPayload {
    pub content: String,
    #[expect(dead_code)]
    pub success: Option<bool>,
}

// The Responses API expects two *different* shapes depending on success vs failure:
//   • success → output is a plain string (no nested object)
//   • failure → output is an object { content, success:false }
// The upstream TypeScript CLI implements this by special‑casing the serialize path.
// We replicate that behavior with a manual Serialize impl.

impl Serialize for FunctionCallOutputPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        // The upstream TypeScript CLI always serializes `output` as a *plain string* regardless
        // of whether the function call succeeded or failed. The boolean is purely informational
        // for local bookkeeping and is NOT sent to the OpenAI endpoint. Sending the nested object
        // form `{ content, success:false }` triggers the 400 we are still seeing. Mirror the JS CLI
        // exactly: always emit a bare string.

        serializer.serialize_str(&self.content)
    }
}

impl<'de> Deserialize<'de> for FunctionCallOutputPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(FunctionCallOutputPayload {
            content: s,
            success: None,
        })
    }
}

// Implement Display so callers can treat the payload like a plain string when logging or doing
// trivial substring checks in tests (existing tests call `.contains()` on the output). Display
// returns the raw `content` field.

impl std::fmt::Display for FunctionCallOutputPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.content)
    }
}

impl std::ops::Deref for FunctionCallOutputPayload {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.content
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::openai_tools::ToJsonSchema;

    #[test]
    fn serializes_success_as_plain_string() {
        let item = ResponseInputItem::FunctionCallOutput {
            call_id: "call1".into(),
            output: FunctionCallOutputPayload {
                content: "ok".into(),
                success: None,
            },
        };

        let json = serde_json::to_string(&item).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Success case -> output should be a plain string
        assert_eq!(v.get("output").unwrap().as_str().unwrap(), "ok");
    }

    #[test]
    fn serializes_failure_as_string() {
        let item = ResponseInputItem::FunctionCallOutput {
            call_id: "call1".into(),
            output: FunctionCallOutputPayload {
                content: "bad".into(),
                success: Some(false),
            },
        };

        let json = serde_json::to_string(&item).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(v.get("output").unwrap().as_str().unwrap(), "bad");
    }

    #[test]
    fn deserialize_shell_tool_call_params() {
        let json = r#"{
            "command": ["ls", "-l"],
            "workdir": "/tmp",
            "timeout": 1000
        }"#;

        let params: ShellToolCallParams = serde_json::from_str(json).unwrap();
        assert_eq!(
            ShellToolCallParams {
                command: vec!["ls".to_string(), "-l".to_string()],
                workdir: Some("/tmp".to_string()),
                timeout: Some(1000),
            },
            params
        );
    }

    #[test]
    fn test_read_file_validation_valid_params() {
        let params = ReadFileToolCallParams {
            path: "test.txt".to_string(),
            should_read_entire_file: false,
            start_line_one_indexed: Some(1),
            end_line_one_indexed_inclusive: Some(10),
            explanation: None,
        };
        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_read_file_validation_start_greater_than_end() {
        let params = ReadFileToolCallParams {
            path: "test.txt".to_string(),
            should_read_entire_file: false,
            start_line_one_indexed: Some(10),
            end_line_one_indexed_inclusive: Some(5),
            explanation: None,
        };
        let result = params.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("start_line_one_indexed (10) must be less than or equal to end_line_one_indexed_inclusive (5)"));
    }

    #[test]
    fn test_read_file_validation_zero_line_numbers() {
        let params = ReadFileToolCallParams {
            path: "test.txt".to_string(),
            should_read_entire_file: false,
            start_line_one_indexed: Some(0),
            end_line_one_indexed_inclusive: Some(10),
            explanation: None,
        };
        let result = params.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("start_line_one_indexed must be greater than 0")
        );
    }

    #[test]
    fn test_read_file_validation_missing_line_numbers() {
        let params = ReadFileToolCallParams {
            path: "test.txt".to_string(),
            should_read_entire_file: false,
            start_line_one_indexed: None,
            end_line_one_indexed_inclusive: Some(10),
            explanation: None,
        };
        let result = params.validate();
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("start_line_one_indexed and end_line_one_indexed_inclusive are required")
        );
    }

    #[test]
    fn test_read_file_validation_empty_path() {
        let params = ReadFileToolCallParams {
            path: "".to_string(),
            should_read_entire_file: true,
            start_line_one_indexed: None,
            end_line_one_indexed_inclusive: None,
            explanation: None,
        };
        let result = params.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("path cannot be empty"));
    }

    #[test]
    fn test_read_file_validation_read_entire_file() {
        let params = ReadFileToolCallParams {
            path: "test.txt".to_string(),
            should_read_entire_file: true,
            start_line_one_indexed: None,
            end_line_one_indexed_inclusive: None,
            explanation: None,
        };
        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_read_file_validation_equal_line_numbers() {
        let params = ReadFileToolCallParams {
            path: "test.txt".to_string(),
            should_read_entire_file: false,
            start_line_one_indexed: Some(5),
            end_line_one_indexed_inclusive: Some(5),
            explanation: None,
        };
        assert!(params.validate().is_ok());
    }
}
