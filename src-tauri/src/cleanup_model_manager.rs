//! Optional verified download for the fully local cleanup GGUF.

use reqwest::blocking::Client;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const MODEL_NAME: &str = "qwen3-4b-instruct-2507-q4_k_m.gguf";
const MODEL_URL: &str = "https://huggingface.co/bartowski/Qwen_Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen_Qwen3-4B-Instruct-2507-Q4_K_M.gguf";
const MODEL_SIZE: u64 = 2_497_280_736;
const MODEL_SHA256: &str = "2fde00ce69dd4899c70d020845e2638353015bba0fdf161b3eb965f2bca4464e";

static STATUS: OnceLock<Mutex<CleanupModelStatus>> = OnceLock::new();
static PROBED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupModelState {
    Missing,
    Verifying,
    Downloading,
    Ready,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanupModelStatus {
    pub state: CleanupModelState,
    pub model: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
}

impl Default for CleanupModelStatus {
    fn default() -> Self {
        Self {
            state: CleanupModelState::Missing,
            model: "Qwen3 4B Q4_K_M".into(),
            downloaded_bytes: installed_bytes(),
            total_bytes: MODEL_SIZE,
            error: None,
        }
    }
}

fn status_cell() -> &'static Mutex<CleanupModelStatus> {
    STATUS.get_or_init(|| Mutex::new(CleanupModelStatus::default()))
}

pub fn model_path() -> PathBuf {
    crate::local_llm::app_support_dir()
        .join("models")
        .join(MODEL_NAME)
}

fn marker_path() -> PathBuf {
    model_path().with_extension("gguf.verified")
}

fn partial_path() -> PathBuf {
    model_path().with_extension("gguf.download")
}

fn installed_bytes() -> u64 {
    model_path()
        .metadata()
        .map(|metadata| metadata.len().min(MODEL_SIZE))
        .unwrap_or(0)
}

fn marker_valid() -> bool {
    fs::read_to_string(marker_path())
        .map(|value| value.trim() == MODEL_SHA256)
        .unwrap_or(false)
        && installed_bytes() == MODEL_SIZE
}

fn update(app: &AppHandle, mutate: impl FnOnce(&mut CleanupModelStatus)) {
    let snapshot = {
        let mut status = status_cell().lock().unwrap();
        mutate(&mut status);
        status.clone()
    };
    let _ = app.emit("cleanup-model-progress", snapshot);
}

fn mark_ready(app: &AppHandle) {
    update(app, |status| {
        status.state = CleanupModelState::Ready;
        status.downloaded_bytes = MODEL_SIZE;
        status.error = None;
    });
}

pub fn status(app: AppHandle) -> CleanupModelStatus {
    if !PROBED.swap(true, Ordering::SeqCst) {
        if marker_valid() {
            mark_ready(&app);
        } else if model_path().exists() {
            update(&app, |status| {
                status.state = CleanupModelState::Verifying;
                status.downloaded_bytes = installed_bytes();
            });
            std::thread::spawn(move || match verify_model() {
                Ok(()) => {
                    let _ = fs::write(marker_path(), MODEL_SHA256);
                    mark_ready(&app);
                    crate::hotkey::reload_local_worker();
                }
                Err(error) => update(&app, |status| {
                    status.state = CleanupModelState::Error;
                    status.error = Some(format!(
                        "Existing cleanup model failed verification: {error}"
                    ));
                }),
            });
        }
    }
    status_cell().lock().unwrap().clone()
}

pub fn start_download(app: AppHandle) -> Result<(), String> {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    CANCELLED.store(false, Ordering::SeqCst);
    update(&app, |status| {
        status.state = CleanupModelState::Downloading;
        status.downloaded_bytes = 0;
        status.error = None;
    });
    std::thread::spawn(move || {
        let result = download(&app);
        RUNNING.store(false, Ordering::SeqCst);
        match result {
            Ok(()) => {
                let _ = fs::write(marker_path(), MODEL_SHA256);
                mark_ready(&app);
                crate::hotkey::reload_local_worker();
            }
            Err(_) if CANCELLED.load(Ordering::SeqCst) => update(&app, |status| {
                status.state = CleanupModelState::Cancelled;
                status.downloaded_bytes = 0;
                status.error = None;
            }),
            Err(error) => {
                let _ = fs::remove_file(partial_path());
                update(&app, |status| {
                    status.state = CleanupModelState::Error;
                    status.downloaded_bytes = 0;
                    status.error = Some(error);
                });
            }
        }
    });
    Ok(())
}

pub fn cancel_download() {
    CANCELLED.store(true, Ordering::SeqCst);
}

fn download(app: &AppHandle) -> Result<(), String> {
    let target = model_path();
    let parent = target
        .parent()
        .ok_or_else(|| "Cleanup model path has no parent".to_string())?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(60 * 60))
        .build()
        .map_err(|error| error.to_string())?;
    let mut response = client
        .get(MODEL_URL)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("Cleanup model download failed: {error}"))?;
    let partial = partial_path();
    let file = File::create(&partial).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 1024 * 256];
    loop {
        if CANCELLED.load(Ordering::SeqCst) {
            drop(writer);
            let _ = fs::remove_file(&partial);
            return Err("cancelled".into());
        }
        let read = response
            .read(&mut buffer)
            .map_err(|error| format!("Cleanup model download interrupted: {error}"))?;
        if read == 0 {
            break;
        }
        writer
            .write_all(&buffer[..read])
            .map_err(|error| error.to_string())?;
        hasher.update(&buffer[..read]);
        downloaded += read as u64;
        update(app, |status| {
            status.downloaded_bytes = downloaded.min(MODEL_SIZE)
        });
    }
    writer.flush().map_err(|error| error.to_string())?;
    if downloaded != MODEL_SIZE {
        let _ = fs::remove_file(&partial);
        return Err(format!(
            "Cleanup model size mismatch: expected {MODEL_SIZE}, received {downloaded}"
        ));
    }
    let digest = format!("{:x}", hasher.finalize());
    if digest != MODEL_SHA256 {
        let _ = fs::remove_file(&partial);
        return Err("Cleanup model checksum verification failed".into());
    }
    fs::rename(&partial, &target).map_err(|error| error.to_string())?;
    Ok(())
}

fn verify_model() -> Result<(), String> {
    let file = File::open(model_path()).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() != MODEL_SIZE {
        return Err("size mismatch".into());
    }
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 256];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = format!("{:x}", hasher.finalize());
    if digest != MODEL_SHA256 {
        return Err("checksum mismatch".into());
    }
    Ok(())
}
