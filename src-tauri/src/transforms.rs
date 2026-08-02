//! Run transform rewrites on selected text via Ollama.

use whimpr_core::transforms::TRANSFORM_SYSTEM;

pub fn apply_via_ollama(
    selected: &str,
    instruction: &str,
    ollama_base: &str,
    ollama_model: &str,
) -> Result<String, String> {
    if selected.trim().is_empty() {
        return Err("Selected text is empty".into());
    }
    let base = ollama_base.trim().trim_end_matches('/');
    let url = if base.is_empty() {
        "http://localhost:11434/v1/chat/completions".to_string()
    } else {
        format!("{base}/chat/completions")
    };

    let user = format!(
        "Instruction: {instruction}\n\n<SELECTED_TEXT>\n{selected}\n</SELECTED_TEXT>"
    );

    let body = serde_json::json!({
        "model": ollama_model,
        "temperature": 0.3,
        "max_tokens": 1200,
        "messages": [
            {"role": "system", "content": TRANSFORM_SYSTEM},
            {"role": "user", "content": user},
        ],
    });

    let out = std::process::Command::new("curl")
        .args(["-sf", "--max-time", "120", "-X", "POST", &url])
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-d")
        .arg(body.to_string())
        .output()
        .map_err(|e| format!("Could not reach Ollama: {e}"))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("Ollama transform failed — is it running? {err}"));
    }

    let v: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|_| "Invalid Ollama response".to_string())?;
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if content.is_empty() {
        return Err("Transform returned empty text".into());
    }
    Ok(content)
}
