//! Verified first-run installation for the primary ASR and VAD models.

use reqwest::blocking::Client;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

const MANIFEST_VERSION: &str = "parakeet-v3-int8+silero-v4-1";
const PARAKEET_DIR: &str = "parakeet-tdt-0.6b-v3-int8";

struct Artifact {
    relative_path: &'static str,
    url: &'static str,
    sha256: &'static str,
    size: u64,
}

const ARTIFACTS: &[Artifact] = &[
    Artifact {
        relative_path: "parakeet-tdt-0.6b-v3-int8/encoder-model.int8.onnx",
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/encoder-model.int8.onnx",
        sha256: "6139d2fa7e1b086097b277c7149725edbab89cc7c7ae64b23c741be4055aff09",
        size: 652_183_999,
    },
    Artifact {
        relative_path: "parakeet-tdt-0.6b-v3-int8/decoder_joint-model.int8.onnx",
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/decoder_joint-model.int8.onnx",
        sha256: "eea7483ee3d1a30375daedc8ed83e3960c91b098812127a0d99d1c8977667a70",
        size: 18_202_004,
    },
    Artifact {
        relative_path: "parakeet-tdt-0.6b-v3-int8/nemo128.onnx",
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/nemo128.onnx",
        sha256: "a9fde1486ebfcc08f328d75ad4610c67835fea58c73ba57e3209a6f6cf019e9f",
        size: 139_764,
    },
    Artifact {
        relative_path: "parakeet-tdt-0.6b-v3-int8/vocab.txt",
        url: "https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx/resolve/main/vocab.txt",
        sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
        size: 93_939,
    },
    Artifact {
        relative_path: "silero_vad_v4.onnx",
        url: "https://huggingface.co/lquint/silero-vad-v4-onnx/resolve/main/silero_vad.onnx",
        sha256: "a35ebf52fd3ce5f1469b2a36158dba761bc47b973ea3382b3186ca15b1f5af28",
        size: 1_807_522,
    },
];

const TOTAL_SIZE: u64 = 672_427_228;

static STATUS: OnceLock<Mutex<ModelDownloadStatus>> = OnceLock::new();
static PROBE_STARTED: AtomicBool = AtomicBool::new(false);
static DOWNLOAD_RUNNING: AtomicBool = AtomicBool::new(false);
static CANCEL_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelDownloadState {
    Missing,
    Verifying,
    Downloading,
    Ready,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelDownloadStatus {
    pub state: ModelDownloadState,
    pub model: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
}

impl Default for ModelDownloadStatus {
    fn default() -> Self {
        Self {
            state: ModelDownloadState::Missing,
            model: "Parakeet v3 + Silero VAD".into(),
            downloaded_bytes: 0,
            total_bytes: TOTAL_SIZE,
            error: None,
        }
    }
}

fn status_cell() -> &'static Mutex<ModelDownloadStatus> {
    STATUS.get_or_init(|| Mutex::new(ModelDownloadStatus::default()))
}

fn models_dir() -> PathBuf {
    crate::local_llm::app_support_dir().join("models")
}

pub fn parakeet_dir() -> PathBuf {
    models_dir().join(PARAKEET_DIR)
}

pub fn vad_path() -> PathBuf {
    models_dir().join("silero_vad_v4.onnx")
}

fn marker_path() -> PathBuf {
    parakeet_dir().join(".verified")
}

fn target_path(artifact: &Artifact) -> PathBuf {
    models_dir().join(artifact.relative_path)
}

fn update(app: &AppHandle, mutate: impl FnOnce(&mut ModelDownloadStatus)) {
    let snapshot = {
        let mut status = status_cell().lock().unwrap();
        mutate(&mut status);
        status.clone()
    };
    let _ = app.emit("model-download-progress", snapshot);
}

pub fn status(app: AppHandle) -> ModelDownloadStatus {
    ensure_probed(app);
    status_cell().lock().unwrap().clone()
}

fn files_have_expected_sizes() -> bool {
    ARTIFACTS.iter().all(|artifact| {
        target_path(artifact)
            .metadata()
            .map(|metadata| metadata.len() == artifact.size)
            .unwrap_or(false)
    })
}

fn installed_bytes() -> u64 {
    ARTIFACTS
        .iter()
        .map(|artifact| {
            target_path(artifact)
                .metadata()
                .map(|metadata| metadata.len().min(artifact.size))
                .unwrap_or(0)
        })
        .sum()
}

fn ensure_probed(app: AppHandle) {
    if PROBE_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let marker_valid = fs::read_to_string(marker_path())
        .map(|value| value.trim() == MANIFEST_VERSION)
        .unwrap_or(false);
    if marker_valid && files_have_expected_sizes() {
        mark_ready(&app);
        return;
    }
    if installed_bytes() == 0 {
        return;
    }

    update(&app, |status| {
        status.state = ModelDownloadState::Verifying;
        status.downloaded_bytes = installed_bytes();
        status.error = None;
    });
    std::thread::spawn(move || match verify_all() {
        Ok(true) => {
            let _ = write_marker();
            mark_ready(&app);
            crate::hotkey::load_asr_model();
        }
        Ok(false) => update(&app, |status| {
            status.state = ModelDownloadState::Error;
            status.error = Some("Existing speech model files failed checksum verification.".into());
        }),
        Err(error) => update(&app, |status| {
            status.state = ModelDownloadState::Error;
            status.error = Some(format!("Could not verify speech models: {error}"));
        }),
    });
}

fn mark_ready(app: &AppHandle) {
    update(app, |status| {
        status.state = ModelDownloadState::Ready;
        status.downloaded_bytes = TOTAL_SIZE;
        status.total_bytes = TOTAL_SIZE;
        status.error = None;
    });
}

pub fn start_download(app: AppHandle) -> Result<(), String> {
    if DOWNLOAD_RUNNING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    CANCEL_REQUESTED.store(false, Ordering::SeqCst);
    update(&app, |status| {
        status.state = ModelDownloadState::Downloading;
        status.downloaded_bytes = 0;
        status.total_bytes = TOTAL_SIZE;
        status.error = None;
    });

    std::thread::spawn(move || {
        let result = download_all(&app);
        DOWNLOAD_RUNNING.store(false, Ordering::SeqCst);
        match result {
            Ok(()) => {
                mark_ready(&app);
                crate::hotkey::load_asr_model();
            }
            Err(_) if CANCEL_REQUESTED.load(Ordering::SeqCst) => update(&app, |status| {
                status.state = ModelDownloadState::Cancelled;
                status.downloaded_bytes = installed_bytes();
                status.error = None;
            }),
            Err(error) => update(&app, |status| {
                status.state = ModelDownloadState::Error;
                status.downloaded_bytes = installed_bytes();
                status.error = Some(error);
            }),
        }
    });
    Ok(())
}

pub fn cancel_download() {
    CANCEL_REQUESTED.store(true, Ordering::SeqCst);
}

fn download_all(app: &AppHandle) -> Result<(), String> {
    fs::create_dir_all(models_dir()).map_err(|e| format!("Could not create models folder: {e}"))?;
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(60 * 60))
        .build()
        .map_err(|e| format!("Could not initialize model download: {e}"))?;
    let mut completed = 0_u64;

    for artifact in ARTIFACTS {
        let target = target_path(artifact);
        if verify_file(&target, artifact.size, artifact.sha256).unwrap_or(false) {
            completed += artifact.size;
            update(app, |status| status.downloaded_bytes = completed);
            continue;
        }
        if CANCEL_REQUESTED.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        download_artifact(&client, app, artifact, completed)?;
        completed += artifact.size;
    }
    write_marker().map_err(|e| format!("Could not record model verification: {e}"))
}

fn download_artifact(
    client: &Client,
    app: &AppHandle,
    artifact: &Artifact,
    completed: u64,
) -> Result<(), String> {
    let target = target_path(artifact);
    let parent = target
        .parent()
        .ok_or_else(|| "Invalid model destination.".to_string())?;
    fs::create_dir_all(parent).map_err(|e| format!("Could not create model folder: {e}"))?;
    let temporary = target.with_extension("download");
    let _ = fs::remove_file(&temporary);

    let mut response = client
        .get(artifact.url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|e| format!("Model download failed: {e}"))?;
    if let Some(size) = response.content_length() {
        if size != artifact.size {
            return Err(format!(
                "Model server returned an unexpected size ({size} bytes)."
            ));
        }
    }

    let file = File::create(&temporary).map_err(|e| format!("Could not create model file: {e}"))?;
    let mut writer = BufWriter::with_capacity(8 * 1024 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut downloaded = 0_u64;
    let mut last_reported = 0_u64;

    loop {
        if CANCEL_REQUESTED.load(Ordering::SeqCst) {
            drop(writer);
            let _ = fs::remove_file(&temporary);
            return Err("cancelled".into());
        }
        let count = response
            .read(&mut buffer)
            .map_err(|e| format!("Model download interrupted: {e}"))?;
        if count == 0 {
            break;
        }
        writer
            .write_all(&buffer[..count])
            .map_err(|e| format!("Could not save model: {e}"))?;
        hasher.update(&buffer[..count]);
        downloaded += count as u64;
        if downloaded.saturating_sub(last_reported) >= 4 * 1024 * 1024 {
            last_reported = downloaded;
            update(app, |status| {
                status.downloaded_bytes = completed + downloaded;
            });
        }
    }
    writer
        .flush()
        .map_err(|e| format!("Could not finish writing model: {e}"))?;
    writer
        .get_ref()
        .sync_all()
        .map_err(|e| format!("Could not sync model to disk: {e}"))?;
    let digest = format!("{:x}", hasher.finalize());
    if downloaded != artifact.size || !digest.eq_ignore_ascii_case(artifact.sha256) {
        drop(writer);
        let _ = fs::remove_file(&temporary);
        return Err("Downloaded model failed checksum verification.".into());
    }
    drop(writer);
    fs::rename(&temporary, &target).map_err(|e| format!("Could not install model: {e}"))
}

fn write_marker() -> std::io::Result<()> {
    fs::create_dir_all(parakeet_dir())?;
    fs::write(marker_path(), format!("{MANIFEST_VERSION}\n"))
}

fn verify_all() -> Result<bool, std::io::Error> {
    for artifact in ARTIFACTS {
        if !verify_file(&target_path(artifact), artifact.size, artifact.sha256)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn verify_file(
    path: &Path,
    expected_size: u64,
    expected_sha: &str,
) -> Result<bool, std::io::Error> {
    let file = File::open(path)?;
    if file.metadata()?.len() != expected_size {
        return Ok(false);
    }
    let mut reader = BufReader::with_capacity(8 * 1024 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()).eq_ignore_ascii_case(expected_sha))
}
