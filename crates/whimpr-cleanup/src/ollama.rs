//! Ollama native `/api/chat` helper. Reasoning models (qwen3, deepseek-r1) return
//! empty `content` on the OpenAI-compatible endpoint unless `think: false` is set
//! on the native API.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

/// True when the base URL or chat-completions URL points at a local Ollama server.
pub fn is_local_ollama(url: &str) -> bool {
    url.contains("11434") || url.contains("localhost:11434")
}

/// Resolve `http://localhost:11434/v1` or `…/v1/chat/completions` → `…/api/chat`.
pub fn native_chat_url(base_or_url: &str) -> String {
    let s = base_or_url.trim().trim_end_matches('/');
    if s.ends_with("/api/chat") {
        return s.to_string();
    }
    if let Some(pos) = s.find("11434") {
        let root = &s[..pos + 5];
        return format!("{root}/api/chat");
    }
    "http://localhost:11434/api/chat".to_string()
}

/// Non-streaming chat via Ollama native API. Sets `think: false` for reasoning models.
pub fn chat(base_or_url: &str, model: &str, messages: &[Value]) -> Result<String> {
    let url = native_chat_url(base_or_url);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("HTTP client")?;

    let body = serde_json::json!({
        "model": model,
        "think": false,
        "stream": false,
        "messages": messages,
        "options": {
            "temperature": 0.2,
            "num_predict": 512,
        }
    });

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .with_context(|| format!("POST {url}"))?;

    let status = resp.status();
    if !status.is_success() {
        let detail = resp.text().unwrap_or_default();
        anyhow::bail!("Ollama HTTP {status}: {detail}");
    }

    let v: Value = resp.json()?;
    let text = v["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        anyhow::bail!("Ollama returned empty content");
    }
    Ok(text)
}

/// Build OpenAI-style `{role, content}` messages from system + user strings.
pub fn simple_messages(system: &str, user: &str) -> Vec<Value> {
    vec![
        serde_json::json!({ "role": "system", "content": system }),
        serde_json::json!({ "role": "user", "content": user }),
    ]
}
