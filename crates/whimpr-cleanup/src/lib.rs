//! Cloud cleanup providers implementing whimpr-core's CleanupProvider trait.
//! OpenAI (default cloud) today; Anthropic and local llama layer in behind the trait.

pub mod ollama;

use std::time::Duration;

use whimpr_core::cleanup::{build_messages, CleanupContext, CleanupProvider, ProviderId};

/// Default OpenAI Chat Completions endpoint.
const OPENAI_DEFAULT_URL: &str = "https://api.openai.com/v1/chat/completions";

/// Cleanup via the OpenAI Chat Completions API — or any OpenAI-compatible
/// endpoint (OpenRouter, a local server, etc.) when `base_url` is set.
/// OpenRouter in particular speaks this exact wire format at
/// `https://openrouter.ai/api/v1/chat/completions`.
pub struct OpenAiProvider {
    client: reqwest::blocking::Client,
    api_key: String,
    model: String,
    /// Full chat-completions URL. Defaults to OpenAI's when empty.
    url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, model: impl Into<String>) -> Self {
        Self::with_base_url(api_key, model, None)
    }

    /// `base_url` is the API root (e.g. `https://openrouter.ai/api/v1`), without
    /// the `/chat/completions` suffix. `None` or empty uses OpenAI directly.
    pub fn with_base_url(
        api_key: String,
        model: impl Into<String>,
        base_url: Option<String>,
    ) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build HTTP client");
        let url = match base_url.map(|s| s.trim().trim_end_matches('/').to_string()) {
            Some(base) if !base.is_empty() => format!("{base}/chat/completions"),
            _ => OPENAI_DEFAULT_URL.to_string(),
        };
        Self {
            client,
            api_key,
            model: model.into(),
            url,
        }
    }
}

impl CleanupProvider for OpenAiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::OpenAi
    }

    fn cleanup(&self, raw: &str, ctx: &CleanupContext) -> anyhow::Result<String> {
        let messages: Vec<serde_json::Value> = build_messages(raw, ctx)
            .into_iter()
            .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
            .collect();

        // Qwen3 / DeepSeek-R1 reasoning models need Ollama's native API + think:false.
        if ollama::is_local_ollama(&self.url) {
            return ollama::chat(&self.url, &self.model, &messages);
        }

        let body = serde_json::json!({
            "model": self.model,
            "temperature": 0.2,
            "max_tokens": 512,
            "messages": messages,
        });

        let mut req = self.client.post(&self.url).json(&body);
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let resp = req.send()?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().unwrap_or_default();
            anyhow::bail!("OpenAI HTTP {status}: {detail}");
        }

        let v: serde_json::Value = resp.json()?;
        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            anyhow::bail!("OpenAI returned empty content");
        }
        Ok(text)
    }
}

/// Cleanup via the Anthropic Messages API. Same shared system prompt; the only
/// difference from OpenAI is the wire envelope (top-level `system`, `x-api-key`).
pub struct AnthropicProvider {
    client: reqwest::blocking::Client,
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, model: impl Into<String>) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build HTTP client");
        Self {
            client,
            api_key,
            model: model.into(),
        }
    }
}

impl CleanupProvider for AnthropicProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Anthropic
    }

    fn cleanup(&self, raw: &str, ctx: &CleanupContext) -> anyhow::Result<String> {
        // Anthropic takes the system prompt top-level; the few-shot turns and the
        // real transcript go in `messages` as user/assistant turns.
        let mut system = String::new();
        let mut messages: Vec<serde_json::Value> = Vec::new();
        for m in build_messages(raw, ctx) {
            if m.role == "system" {
                system = m.content;
            } else {
                messages.push(serde_json::json!({ "role": m.role, "content": m.content }));
            }
        }
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 512,
            "temperature": 0.2,
            "system": system,
            "messages": messages,
        });

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()?;

        let status = resp.status();
        if !status.is_success() {
            let detail = resp.text().unwrap_or_default();
            anyhow::bail!("Anthropic HTTP {status}: {detail}");
        }

        let v: serde_json::Value = resp.json()?;
        let text = v["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            anyhow::bail!("Anthropic returned empty content");
        }
        Ok(text)
    }
}
