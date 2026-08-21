use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

/// A tool definition in a provider-agnostic form.
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Provider-agnostic conversation message.
#[derive(Clone)]
pub enum NormMessage {
    User(String),
    Assistant {
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// One turn from the model: a text reply and/or a set of tool calls.
pub struct LlmTurn {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
}

/// LLM client. `Anthropic` speaks the Messages API; `OpenAi` speaks the
/// OpenAI Chat Completions API (used for Ollama and any OpenAI-compatible server).
pub enum LlmClient {
    Anthropic {
        http: Client,
        api_key: String,
        base_url: String,
        model: String,
    },
    OpenAi {
        http: Client,
        base_url: String,
        api_key: Option<String>,
        model: String,
    },
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("failed to build HTTP client")
}

impl LlmClient {
    /// Build the client from environment variables.
    /// Returns `None` when no usable provider is configured (offline mode).
    pub fn new() -> Option<Self> {
        let provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());
        match provider.as_str() {
            "ollama" => {
                let base_url = std::env::var("OLLAMA_BASE_URL")
                    .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
                let model =
                    std::env::var("OLLAMA_MODEL").unwrap_or_else(|_| "qwen3:8b".to_string());
                Some(LlmClient::OpenAi {
                    http: http_client(),
                    base_url,
                    api_key: None,
                    model,
                })
            }
            "openai" => {
                let base_url = std::env::var("OPENAI_BASE_URL")
                    .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
                let api_key = std::env::var("OPENAI_API_KEY")
                    .ok()
                    .filter(|s| !s.is_empty());
                let model =
                    std::env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
                api_key.map(|k| LlmClient::OpenAi {
                    http: http_client(),
                    base_url,
                    api_key: Some(k),
                    model,
                })
            }
            _ => {
                // anthropic (default)
                let api_key = std::env::var("ANTHROPIC_API_KEY")
                    .ok()
                    .filter(|s| !s.is_empty());
                let base_url = std::env::var("ANTHROPIC_BASE_URL")
                    .unwrap_or_else(|_| "https://api.anthropic.com".to_string());
                let model = std::env::var("SCHEDULER_MODEL")
                    .unwrap_or_else(|_| "claude-sonnet-5".to_string());
                api_key.map(|k| LlmClient::Anthropic {
                    http: http_client(),
                    api_key: k,
                    base_url,
                    model,
                })
            }
        }
    }

    /// Human-readable label for startup logging.
    pub fn label(&self) -> String {
        match self {
            LlmClient::Anthropic { model, .. } => format!("Anthropic ({model})"),
            LlmClient::OpenAi { model, api_key, .. } => {
                if api_key.is_none() {
                    format!("Ollama ({model})")
                } else {
                    format!("OpenAI-compatible ({model})")
                }
            }
        }
    }

    /// Run one model turn with tool-use.
    pub async fn chat(
        &self,
        system: &str,
        convo: &[NormMessage],
        tools: &[ToolDef],
    ) -> Result<LlmTurn, String> {
        match self {
            LlmClient::Anthropic {
                http,
                api_key,
                base_url,
                model,
            } => anthropic_chat(http, api_key, base_url, model, system, convo, tools).await,
            LlmClient::OpenAi {
                http,
                base_url,
                api_key,
                model,
            } => openai_chat(http, base_url, api_key.as_deref(), model, system, convo, tools).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Anthropic Messages API
// ---------------------------------------------------------------------------

fn anthropic_messages(convo: &[NormMessage]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < convo.len() {
        match &convo[i] {
            NormMessage::User(t) => {
                out.push(json!({ "role": "user", "content": t }));
                i += 1;
            }
            NormMessage::Assistant { text, tool_calls } => {
                let mut content = Vec::new();
                if !text.is_empty() {
                    content.push(json!({ "type": "text", "text": text }));
                }
                for tc in tool_calls {
                    content.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }
                out.push(json!({ "role": "assistant", "content": content }));
                i += 1;
            }
            NormMessage::ToolResult { .. } => {
                let mut results = Vec::new();
                while i < convo.len() {
                    if let NormMessage::ToolResult {
                        tool_call_id,
                        content,
                    } = &convo[i]
                    {
                        results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": tool_call_id,
                            "content": content,
                        }));
                        i += 1;
                    } else {
                        break;
                    }
                }
                out.push(json!({ "role": "user", "content": results }));
            }
        }
    }
    out
}

async fn anthropic_chat(
    http: &Client,
    api_key: &str,
    base_url: &str,
    model: &str,
    system: &str,
    convo: &[NormMessage],
    tools: &[ToolDef],
) -> Result<LlmTurn, String> {
    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect();
    let body = json!({
        "model": model,
        "max_tokens": 1024,
        "system": system,
        "messages": anthropic_messages(convo),
        "tools": tools_json,
    });
    let url = format!("{base_url}/v1/messages");
    let resp = http
        .post(&url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
    if !status.is_success() {
        return Err(format!("Anthropic API {status}: {text}"));
    }
    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| format!("parse response: {e} :: {text}"))?;

    let mut out_text = String::new();
    let mut tool_calls = Vec::new();
    if let Some(arr) = parsed.get("content").and_then(|v| v.as_array()) {
        for b in arr {
            match b.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                        out_text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let name = b.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let input = b.get("input").cloned().unwrap_or(Value::Null);
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments: input,
                    });
                }
                _ => {}
            }
        }
    }
    Ok(LlmTurn {
        text: out_text,
        tool_calls,
    })
}

// ---------------------------------------------------------------------------
// OpenAI Chat Completions API (Ollama / OpenAI-compatible)
// ---------------------------------------------------------------------------

fn openai_messages(system: &str, convo: &[NormMessage]) -> Vec<Value> {
    let mut msgs = vec![json!({ "role": "system", "content": system })];
    for m in convo {
        match m {
            NormMessage::User(t) => msgs.push(json!({ "role": "user", "content": t })),
            NormMessage::Assistant { text, tool_calls } => {
                let mut entry = json!({
                    "role": "assistant",
                    "content": if text.is_empty() { Value::Null } else { json!(text) },
                });
                if !tool_calls.is_empty() {
                    let calls: Vec<Value> = tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments.to_string(),
                                }
                            })
                        })
                        .collect();
                    entry["tool_calls"] = json!(calls);
                }
                msgs.push(entry);
            }
            NormMessage::ToolResult {
                tool_call_id,
                content,
            } => msgs.push(json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": content,
            })),
        }
    }
    msgs
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiRespMessage,
}

#[derive(Deserialize)]
struct OpenAiRespMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Deserialize)]
struct OpenAiToolCall {
    id: String,
    function: OpenAiFunction,
}

#[derive(Deserialize)]
struct OpenAiFunction {
    name: String,
    arguments: String,
}

async fn openai_chat(
    http: &Client,
    base_url: &str,
    api_key: Option<&str>,
    model: &str,
    system: &str,
    convo: &[NormMessage],
    tools: &[ToolDef],
) -> Result<LlmTurn, String> {
    let tools_json: Vec<Value> = tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })
        })
        .collect();
    let body = json!({
        "model": model,
        "messages": openai_messages(system, convo),
        "tools": tools_json,
        "tool_choice": "auto",
        "stream": false,
    });
    let url = format!("{base_url}/chat/completions");
    let mut req = http.post(&url).header("content-type", "application/json");
    if let Some(key) = api_key {
        req = req.header("Authorization", format!("Bearer {key}"));
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
    if !status.is_success() {
        return Err(format!("OpenAI-compatible API {status}: {text}"));
    }
    let parsed: OpenAiResponse =
        serde_json::from_str(&text).map_err(|e| format!("parse response: {e} :: {text}"))?;

    let msg = parsed
        .choices
        .into_iter()
        .next()
        .ok_or_else(|| "no choices in response".to_string())?
        .message;

    let mut tool_calls = Vec::new();
    if let Some(calls) = msg.tool_calls {
        for c in calls {
            let args: Value = serde_json::from_str(&c.function.arguments).unwrap_or(Value::Null);
            tool_calls.push(ToolCall {
                id: c.id,
                name: c.function.name,
                arguments: args,
            });
        }
    }
    Ok(LlmTurn {
        text: msg.content.unwrap_or_default(),
        tool_calls,
    })
}
