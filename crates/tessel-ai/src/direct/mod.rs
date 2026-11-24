use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tessel_core::SceneState;

use crate::error::AiError;
use crate::tools::tool_definitions;
use crate::translator::{palette_summary_json, scene_summary_json};

pub const MAX_TOOL_TURNS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum AiProviderKind {
    #[default]
    OpenAi,
    Ollama,
    Gemini,
    Anthropic,
}

impl AiProviderKind {
    pub const ALL: [Self; 4] = [Self::OpenAi, Self::Ollama, Self::Gemini, Self::Anthropic];

    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenAi => "OpenAI",
            Self::Ollama => "Ollama (Local)",
            Self::Gemini => "Gemini",
            Self::Anthropic => "Claude",
        }
    }

    pub const fn default_model(self) -> &'static str {
        match self {
            Self::OpenAi => "gpt-4o-mini",
            Self::Ollama => "llama3.2",
            Self::Gemini => "gemini-2.5-flash",
            Self::Anthropic => "claude-3-5-sonnet-latest",
        }
    }

    pub const fn default_base_url(self) -> &'static str {
        match self {
            Self::OpenAi => "https://api.openai.com/v1/chat/completions",
            Self::Ollama => "http://127.0.0.1:11434/v1/chat/completions",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta",
            Self::Anthropic => "https://api.anthropic.com/v1/messages",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectAiConfig {
    pub provider: AiProviderKind,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
}

impl Default for DirectAiConfig {
    fn default() -> Self {
        Self {
            provider: AiProviderKind::Ollama,
            model: AiProviderKind::Ollama.default_model().into(),
            base_url: "http://127.0.0.1:11434/v1/chat/completions".into(),
            api_key: String::new(),
        }
    }
}

/// Planned tool call emitted by an LLM provider.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedToolCall {
    pub name: String,
    pub arguments: Value,
}

/// Result returned by Direct AI containing both textual reply and validated tool calls to execute.
#[derive(Debug, Clone, PartialEq)]
pub struct DirectAiResponse {
    pub reply: String,
    pub tool_calls: Vec<PlannedToolCall>,
}

/// Builds the compact, deterministic Tessel system prompt with current scene summary.
pub fn build_system_prompt(scene: &SceneState) -> String {
    format!(
        "You are Tessel Direct AI, a 3D/2D asset co-creator.\n\
         Coordinate system: Right-Handed, +Y Up, +Z Front, 1 voxel = 0.1m.\n\
         Active Scene Summary: {}\n\
         Active Palette: {}\n\
         Use the provided tools directly to build or modify voxels, low-poly meshes, rigs, or exports.",
        scene_summary_json(scene),
        palette_summary_json(scene),
    )
}

fn openai_tools_payload() -> Vec<Value> {
    tool_definitions()
        .into_iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t["name"],
                    "description": t["description"],
                    "parameters": t["inputSchema"]
                }
            })
        })
        .collect()
}

/// Extracts OpenAI-style `tool_calls` from a chat completion message JSON value.
/// Returns `Err(AiError::BatchLimitExceeded)` if the provider returns more than `MAX_TOOL_TURNS` calls.
pub fn parse_openai_tool_calls(message: &Value) -> Result<Vec<PlannedToolCall>, AiError> {
    let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if calls.len() > MAX_TOOL_TURNS {
        return Err(AiError::BatchLimitExceeded);
    }
    let mut out = Vec::with_capacity(calls.len());
    for call in calls {
        let Some(func) = call.get("function") else {
            continue;
        };
        let Some(name) = func.get("name").and_then(Value::as_str) else {
            continue;
        };
        let arguments = match func.get("arguments") {
            Some(Value::String(raw)) => serde_json::from_str(raw).unwrap_or_else(|_| json!({})),
            Some(other) => other.clone(),
            None => json!({}),
        };
        out.push(PlannedToolCall {
            name: name.to_string(),
            arguments,
        });
    }
    Ok(out)
}

/// Resolves the final HTTP endpoint URL for any provider, honoring custom `base_url`.
pub fn resolve_endpoint(config: &DirectAiConfig) -> String {
    let base = if config.base_url.trim().is_empty() {
        config.provider.default_base_url()
    } else {
        config.base_url.trim().trim_end_matches('/')
    };

    match config.provider {
        AiProviderKind::OpenAi | AiProviderKind::Ollama | AiProviderKind::Anthropic => {
            base.to_string()
        }
        AiProviderKind::Gemini => {
            format!(
                "{base}/models/{}:generateContent?key={}",
                config.model, config.api_key
            )
        }
    }
}

/// Sends a real HTTP chat completion request with Tessel tool definitions attached.
/// Surfaces real network/auth errors with zero mocking.
pub async fn send_prompt(
    config: &DirectAiConfig,
    scene: &SceneState,
    user_prompt: &str,
) -> Result<DirectAiResponse, AiError> {
    if config.provider != AiProviderKind::Ollama && config.api_key.trim().is_empty() {
        return Err(AiError::MissingApiKey);
    }

    let endpoint = resolve_endpoint(config);

    let system_prompt = build_system_prompt(scene);
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    let response = match config.provider {
        AiProviderKind::OpenAi | AiProviderKind::Ollama => {
            let mut req = client.post(&endpoint).json(&json!({
                "model": config.model,
                "messages": [
                    { "role": "system", "content": system_prompt },
                    { "role": "user", "content": user_prompt }
                ],
                "tools": openai_tools_payload()
            }));
            if !config.api_key.trim().is_empty() {
                req = req.bearer_auth(&config.api_key);
            }
            req.send().await?
        }
        AiProviderKind::Gemini => {
            client
                .post(&endpoint)
                .json(&json!({
                    "system_instruction": { "parts": [{ "text": system_prompt }] },
                    "contents": [{ "parts": [{ "text": user_prompt }] }]
                }))
                .send()
                .await?
        }
        AiProviderKind::Anthropic => {
            client
                .post(&endpoint)
                .header("x-api-key", &config.api_key)
                .header("anthropic-version", "2023-06-01")
                .json(&json!({
                    "model": config.model,
                    "max_tokens": 1024,
                    "system": system_prompt,
                    "messages": [{ "role": "user", "content": user_prompt }]
                }))
                .send()
                .await?
        }
    };

    let status = response.status();
    let body_text = response.text().await?;
    tracing::info!(
        provider = ?config.provider,
        model = %config.model,
        status = status.as_u16(),
        "Direct AI provider response received"
    );
    if !status.is_success() {
        return Err(AiError::ProviderStatus {
            status: status.as_u16(),
            message: body_text,
        });
    }

    let val: Value = serde_json::from_str(&body_text)?;
    match config.provider {
        AiProviderKind::OpenAi | AiProviderKind::Ollama => {
            let msg = &val["choices"][0]["message"];
            let reply = msg["content"].as_str().unwrap_or("").to_string();
            let tool_calls = parse_openai_tool_calls(msg)?;
            Ok(DirectAiResponse { reply, tool_calls })
        }
        AiProviderKind::Gemini => {
            let reply = val["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_string();
            Ok(DirectAiResponse {
                reply,
                tool_calls: Vec::new(),
            })
        }
        AiProviderKind::Anthropic => {
            let reply = val["content"][0]["text"].as_str().unwrap_or("").to_string();
            Ok(DirectAiResponse {
                reply,
                tool_calls: Vec::new(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_openai_tool_calls_rejects_excess_tool_turns() {
        let mut calls = Vec::new();
        for i in 0..12 {
            calls.push(json!({
                "id": format!("call_{i}"),
                "type": "function",
                "function": {
                    "name": "voxel_fill_box",
                    "arguments": "{\"min\":[0,0,0],\"max\":[1,1,1],\"palette_index\":4}"
                }
            }));
        }
        let msg = json!({ "content": null, "tool_calls": calls });
        assert!(matches!(
            parse_openai_tool_calls(&msg),
            Err(AiError::BatchLimitExceeded)
        ));

        let ok_msg = json!({ "content": null, "tool_calls": &msg["tool_calls"].as_array().unwrap()[..MAX_TOOL_TURNS] });
        let parsed = parse_openai_tool_calls(&ok_msg).unwrap();
        assert_eq!(parsed.len(), MAX_TOOL_TURNS);
        assert_eq!(parsed[0].name, "voxel_fill_box");
        assert_eq!(parsed[0].arguments["palette_index"], 4);
    }

    #[test]
    fn resolve_endpoint_honors_custom_base_urls_for_all_providers() {
        let openai_custom = DirectAiConfig {
            provider: AiProviderKind::OpenAi,
            model: "llama-3.3-70b".into(),
            base_url: "https://openrouter.ai/api/v1/chat/completions/".into(),
            api_key: "sk-test".into(),
        };
        assert_eq!(
            resolve_endpoint(&openai_custom),
            "https://openrouter.ai/api/v1/chat/completions"
        );

        let gemini_default = DirectAiConfig {
            provider: AiProviderKind::Gemini,
            model: "gemini-2.5-flash".into(),
            base_url: String::new(),
            api_key: "gk".into(),
        };
        assert_eq!(
            resolve_endpoint(&gemini_default),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent?key=gk"
        );
    }
}
