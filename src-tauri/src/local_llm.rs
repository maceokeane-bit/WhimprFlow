//! Spawns and talks to the local-LLM cleanup worker (a separate process, so
//! llama.cpp and whisper.cpp never link into the same binary). One JSON request
//! per line over stdio: `{system,user}` -> `{text}`.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct LocalWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl LocalWorker {
    pub fn spawn(worker_bin: &Path, model: &Path) -> anyhow::Result<Self> {
        let mut child = Command::new(worker_bin)
            .arg(model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        let stdout = BufReader::new(child.stdout.take().ok_or_else(|| anyhow::anyhow!("no stdout"))?);
        Ok(Self { child, stdin, stdout })
    }

    /// Send one cleanup request (system prompt + few-shot turns + transcript) and
    /// read the response (blocks until the line comes).
    pub fn cleanup(
        &mut self,
        messages: &[whimpr_core::cleanup::CleanupMsg],
    ) -> anyhow::Result<String> {
        let req = serde_json::json!({ "messages": messages, "max_tokens": 400 });
        let mut line = serde_json::to_string(&req)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()?;

        let mut resp = String::new();
        if self.stdout.read_line(&mut resp)? == 0 {
            anyhow::bail!("local worker closed");
        }
        let v: serde_json::Value = serde_json::from_str(&resp)?;
        if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            anyhow::bail!("local llm: {err}");
        }
        Ok(v.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string())
    }
}

impl Drop for LocalWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Platform application-support dir: `~/Library/Application Support/WhimprFlow`
/// on macOS, `%APPDATA%\WhimprFlow` on Windows.
pub fn app_support_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var("APPDATA").unwrap_or_default();
        PathBuf::from(base).join("WhimprFlow")
    }
    #[cfg(not(target_os = "windows"))]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join("Library/Application Support/WhimprFlow")
    }
}

/// Find the worker binary: next to the app executable (bundled), else common dev paths.
pub fn worker_bin_path() -> Option<PathBuf> {
    let exe_name = if cfg!(target_os = "windows") {
        "whimpr-llm-worker.exe"
    } else {
        "whimpr-llm-worker"
    };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join(exe_name);
            if cand.exists() {
                return Some(cand);
            }
        }
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("target/release").join(exe_name));
        candidates.push(cwd.join("target/debug").join(exe_name));
    }
    if let Ok(home) = std::env::var("HOME") {
        candidates.push(
            PathBuf::from(&home)
                .join("Projects/WhimprFlow/target/release")
                .join(exe_name),
        );
        candidates.push(
            PathBuf::from(&home)
                .join("WhimprFlow/target/release")
                .join(exe_name),
        );
    }
    candidates.into_iter().find(|p| p.exists())
}

/// Known-good GGUF filenames for cleanup, highest quality first.
const PREFERRED_GGUF: &[&str] = &[
    "qwen3-4b-instruct-2507-q4_k_m.gguf",
    "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
    "qwen3-8b-instruct-q4_k_m.gguf",
    "Qwen3-8B-Instruct-Q4_K_M.gguf",
    "qwen3-1.7b-instruct-q4_k_m.gguf",
    "Qwen3-1.7B-Instruct-Q4_K_M.gguf",
    "qwen2.5-1.5b-instruct-q4_k_m.gguf",
    "Qwen2.5-1.5B-Instruct-Q4_K_M.gguf",
];

/// Resolve the local cleanup GGUF: settings override → preferred names → any `.gguf` in `models/`.
pub fn model_path() -> Option<PathBuf> {
    let dir = app_support_dir().join("models");
    let settings = whimpr_core::Settings::load(&app_support_dir().join("settings.json"));
    if !settings.local_model.is_empty() {
        let p = dir.join(&settings.local_model);
        if p.exists() {
            return Some(p);
        }
        eprintln!(
            "[whimpr] local_model setting '{}' not found in {}",
            settings.local_model,
            dir.display()
        );
    }
    for name in PREFERRED_GGUF {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut ggufs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "gguf"))
            .collect();
        ggufs.sort();
        if let Some(p) = ggufs.first() {
            eprintln!(
                "[whimpr] using auto-detected GGUF {}",
                p.file_name().unwrap_or_default().to_string_lossy()
            );
            return Some(p.clone());
        }
    }
    None
}

/// Spawn the worker if both the binary and a GGUF model are present.
pub fn spawn_default() -> Option<LocalWorker> {
    let bin = worker_bin_path()?;
    let model = model_path()?;
    match LocalWorker::spawn(&bin, &model) {
        Ok(w) => {
            eprintln!(
                "[whimpr] local LLM worker started ({}, model={})",
                bin.display(),
                model.display()
            );
            Some(w)
        }
        Err(e) => {
            eprintln!("[whimpr] local LLM worker failed to start: {e}");
            None
        }
    }
}
