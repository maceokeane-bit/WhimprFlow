//! Persist short dictation WAVs next to stats, pruned with history retention.

use std::path::{Path, PathBuf};

/// Write 16 kHz mono f32 PCM as a 16-bit WAV. Returns the absolute path on success.
pub fn write_wav(path: &Path, pcm16k: &[f32]) -> std::io::Result<PathBuf> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    for &sample in pcm16k {
        let clamped = sample.clamp(-1.0, 1.0);
        let i = (clamped * i16::MAX as f32).round() as i16;
        writer
            .write_sample(i)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    }
    writer
        .finalize()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    Ok(path.to_path_buf())
}

pub fn delete_file(path: &str) {
    let p = Path::new(path);
    if p.exists() {
        let _ = std::fs::remove_file(p);
    }
}

pub fn read_bytes(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}
