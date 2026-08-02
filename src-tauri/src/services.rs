//! Local service health + controls (Ollama, ASR model on disk, GGUF backup).

use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

use crate::local_llm;

#[derive(Debug, Clone, Serialize)]
pub struct ServicesStatus {
    /// Ollama HTTP API responding on localhost:11434.
    pub ollama_running: bool,
    /// Tags returned by `ollama list` / /api/tags (empty if Ollama is down).
    pub ollama_models: Vec<String>,
    /// Primary or fallback ASR model present on disk.
    pub asr_ready: bool,
    pub asr_model: Option<String>,
    /// GGUF cleanup backup present on disk.
    pub gguf_ready: bool,
    pub gguf_model: Option<String>,
    /// llama.cpp worker binary built and findable.
    pub local_worker_ready: bool,
    /// Speech model loaded in memory (WhimprFlow must be running).
    pub asr_loaded: bool,
}

fn models_dir() -> PathBuf {
    local_llm::app_support_dir().join("models")
}

fn asr_on_disk() -> Option<String> {
    let dir = models_dir();
    if dir
        .join("parakeet-tdt-0.6b-v3-int8/encoder-model.int8.onnx")
        .exists()
    {
        return Some("Parakeet TDT 0.6B v3 (int8)".into());
    }
    for name in [
        "ggml-large-v3-turbo.bin",
        "ggml-medium.en.bin",
        "ggml-small.en.bin",
        "ggml-base.en.bin",
    ] {
        let p = dir.join(name);
        if p.exists() {
            return Some(name.to_string());
        }
    }
    None
}

/// Ping Ollama and list installed model tags.
pub fn ollama_status() -> (bool, Vec<String>) {
    let out = Command::new("curl")
        .args(["-sf", "--max-time", "2", "http://localhost:11434/api/tags"])
        .output();
    let Ok(out) = out else {
        return (false, Vec::new());
    };
    if !out.status.success() {
        return (false, Vec::new());
    }
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return (true, Vec::new());
    };
    let models = v["models"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    (true, models)
}

pub fn status(asr_loaded: bool) -> ServicesStatus {
    let (ollama_running, ollama_models) = ollama_status();
    let asr_model = asr_on_disk();
    let gguf_path = local_llm::model_path();
    let gguf_model = gguf_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    ServicesStatus {
        ollama_running,
        ollama_models,
        asr_ready: asr_model.is_some(),
        asr_model,
        gguf_ready: gguf_path.is_some(),
        gguf_model,
        local_worker_ready: local_llm::worker_bin_path().is_some(),
        asr_loaded,
    }
}

/// Launch the Ollama menubar app (starts the local API server on macOS).
#[cfg(target_os = "macos")]
pub fn start_ollama() -> Result<(), String> {
    Command::new("open")
        .arg("-a")
        .arg("Ollama")
        .spawn()
        .map_err(|e| format!("could not open Ollama: {e}"))?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn start_ollama() -> Result<(), String> {
    Command::new("ollama")
        .arg("serve")
        .spawn()
        .map_err(|e| format!("could not start ollama serve: {e}"))?;
    Ok(())
}

/// Pull a model tag in the background (`ollama pull …`).
pub fn pull_ollama_model(model: &str) -> Result<(), String> {
    let model = model.trim();
    if model.is_empty() {
        return Err("model name is empty".into());
    }
    Command::new("ollama")
        .args(["pull", model])
        .spawn()
        .map_err(|e| format!("could not run ollama pull: {e}"))?;
    Ok(())
}
