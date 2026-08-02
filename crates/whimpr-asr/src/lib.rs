//! Local speech-to-text via whisper.cpp (whisper-rs), implementing
//! [`whimpr_core::AsrEngine`]. Expects 16 kHz mono f32 samples.

use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

use transcribe_rs::onnx::parakeet::ParakeetModel;
use transcribe_rs::onnx::Quantization;
use transcribe_rs::vad::{SileroVad, SmoothedVad, Vad};
use transcribe_rs::{SpeechModel, TranscribeOptions};
use whimpr_core::asr::{AsrCaps, AsrEngine, AsrEngineId, Transcript};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// A loaded whisper model ready to transcribe utterances.
pub struct WhisperEngine {
    ctx: WhisperContext,
    language: RwLock<String>,
}

impl WhisperEngine {
    /// Load a GGML/GGUF whisper model from `model_path`.
    pub fn load(model_path: &Path) -> anyhow::Result<Self> {
        let path = model_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("model path is not valid UTF-8"))?;
        let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
            .map_err(|e| anyhow::anyhow!("failed to load whisper model: {e}"))?;
        Ok(Self {
            ctx,
            language: RwLock::new("en".into()),
        })
    }
}

impl AsrEngine for WhisperEngine {
    fn set_language(&self, language: &str) -> anyhow::Result<()> {
        *self
            .language
            .write()
            .map_err(|_| anyhow::anyhow!("Whisper language lock poisoned"))? = language.to_string();
        Ok(())
    }

    fn id(&self) -> AsrEngineId {
        AsrEngineId::WhisperCpp
    }

    fn caps(&self) -> AsrCaps {
        AsrCaps {
            supports_streaming: false,
        }
    }

    fn transcribe(&self, pcm16k: &[f32]) -> anyhow::Result<Transcript> {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| anyhow::anyhow!("whisper create_state: {e}"))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        let language = self
            .language
            .read()
            .map_err(|_| anyhow::anyhow!("Whisper language lock poisoned"))?
            .clone();
        params.set_language(Some(&language));
        params.set_translate(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_suppress_blank(true);
        // Push-to-talk utterances are always one short clip, not long-form audio.
        // Without this, whisper.cpp can split it into multiple internal segments
        // that repeat the same words — which then get concatenated below,
        // producing the sentence twice. Single-segment mode avoids that.
        params.set_single_segment(true);
        params.set_no_context(true);

        state
            .full(params, pcm16k)
            .map_err(|e| anyhow::anyhow!("whisper full: {e}"))?;

        let n = state
            .full_n_segments()
            .map_err(|e| anyhow::anyhow!("whisper n_segments: {e}"))?;
        let mut text = String::new();
        for i in 0..n {
            if let Ok(seg) = state.full_get_segment_text(i) {
                text.push_str(&seg);
            }
        }

        Ok(Transcript {
            text: text.trim().to_string(),
            confidence: None,
        })
    }
}

/// ONNX Parakeet TDT backend. The model API is mutable because it reuses ONNX
/// sessions and decoder state, so a mutex keeps the shared `AsrEngine` seam safe.
pub struct ParakeetEngine {
    model: Mutex<ParakeetModel>,
    language: RwLock<String>,
}

impl ParakeetEngine {
    pub fn load(model_dir: &Path) -> anyhow::Result<Self> {
        let model = ParakeetModel::load(model_dir, &Quantization::Int8)
            .map_err(|e| anyhow::anyhow!("failed to load Parakeet model: {e}"))?;
        Ok(Self {
            model: Mutex::new(model),
            language: RwLock::new("en".into()),
        })
    }
}

impl AsrEngine for ParakeetEngine {
    fn set_language(&self, language: &str) -> anyhow::Result<()> {
        *self
            .language
            .write()
            .map_err(|_| anyhow::anyhow!("Parakeet language lock poisoned"))? =
            language.to_string();
        Ok(())
    }

    fn id(&self) -> AsrEngineId {
        AsrEngineId::OnnxParakeet
    }

    fn transcribe(&self, pcm16k: &[f32]) -> anyhow::Result<Transcript> {
        let language = self
            .language
            .read()
            .map_err(|_| anyhow::anyhow!("Parakeet language lock poisoned"))?
            .clone();
        let result = self
            .model
            .lock()
            .map_err(|_| anyhow::anyhow!("Parakeet model lock poisoned"))?
            .transcribe(
                pcm16k,
                &TranscribeOptions {
                    language: Some(language),
                    ..Default::default()
                },
            )
            .map_err(|e| anyhow::anyhow!("Parakeet transcription failed: {e}"))?;
        Ok(Transcript {
            text: result.text.trim().to_string(),
            confidence: None,
        })
    }
}

/// Ordered ASR fallback chain. A runtime inference error or empty result from
/// Parakeet automatically falls through to Whisper instead of losing dictation.
pub struct FallbackEngine {
    engines: Vec<Arc<dyn AsrEngine>>,
}

impl FallbackEngine {
    pub fn new(engines: Vec<Arc<dyn AsrEngine>>) -> anyhow::Result<Self> {
        if engines.is_empty() {
            anyhow::bail!("no ASR engines are available");
        }
        Ok(Self { engines })
    }
}

impl AsrEngine for FallbackEngine {
    fn set_language(&self, language: &str) -> anyhow::Result<()> {
        for engine in &self.engines {
            engine.set_language(language)?;
        }
        Ok(())
    }

    fn id(&self) -> AsrEngineId {
        self.engines[0].id()
    }

    fn caps(&self) -> AsrCaps {
        self.engines[0].caps()
    }

    fn warmup(&self) -> anyhow::Result<()> {
        self.engines[0].warmup()
    }

    fn transcribe(&self, pcm16k: &[f32]) -> anyhow::Result<Transcript> {
        let mut last_error = None;
        for engine in &self.engines {
            match engine.transcribe(pcm16k) {
                Ok(transcript) if !transcript.text.trim().is_empty() => return Ok(transcript),
                Ok(_) => {
                    last_error = Some(anyhow::anyhow!(
                        "{:?} returned an empty transcript",
                        engine.id()
                    ));
                }
                Err(error) => {
                    last_error = Some(anyhow::anyhow!("{:?} failed: {error}", engine.id()));
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("all ASR engines failed")))
    }
}

/// Stateful Silero gate reused between utterances. It trims non-speech while
/// retaining 450 ms of pre-roll and hangover around detected speech.
pub struct SileroVadTrimmer {
    vad: Mutex<SmoothedVad>,
}

impl SileroVadTrimmer {
    pub fn load(model_path: &Path) -> anyhow::Result<Self> {
        let silero = SileroVad::new(model_path, 0.3)
            .map_err(|e| anyhow::anyhow!("failed to load Silero VAD: {e}"))?;
        Ok(Self {
            vad: Mutex::new(SmoothedVad::new(Box::new(silero), 15, 15, 2)),
        })
    }

    pub fn trim(&self, pcm16k: &[f32]) -> anyhow::Result<Vec<f32>> {
        let mut vad = self
            .vad
            .lock()
            .map_err(|_| anyhow::anyhow!("Silero VAD lock poisoned"))?;
        vad.reset();
        trim_with_vad(&mut *vad, pcm16k)
    }
}

fn trim_with_vad(vad: &mut dyn Vad, pcm16k: &[f32]) -> anyhow::Result<Vec<f32>> {
    let frame_size = vad.frame_size();
    let mut speech = Vec::with_capacity(pcm16k.len());
    let mut was_speech = false;

    for frame in pcm16k.chunks_exact(frame_size) {
        let in_speech = vad
            .is_speech(frame)
            .map_err(|e| anyhow::anyhow!("VAD inference failed: {e}"))?;
        if in_speech && !was_speech {
            speech.extend(vad.drain_prefill());
        }
        if in_speech {
            speech.extend_from_slice(frame);
        }
        was_speech = in_speech;
    }

    // A failed/overly-strict VAD decision must never erase a dictation.
    if speech.is_empty() {
        Ok(pcm16k.to_vec())
    } else {
        Ok(speech)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transcribe_rs::vad::EnergyVad;

    struct FakeEngine {
        id: AsrEngineId,
        text: Option<&'static str>,
    }

    impl AsrEngine for FakeEngine {
        fn id(&self) -> AsrEngineId {
            self.id
        }

        fn transcribe(&self, _pcm16k: &[f32]) -> anyhow::Result<Transcript> {
            match self.text {
                Some(text) => Ok(Transcript {
                    text: text.into(),
                    confidence: None,
                }),
                None => anyhow::bail!("inference failed"),
            }
        }
    }

    #[test]
    fn asr_chain_falls_back_after_runtime_failure() {
        let chain = FallbackEngine::new(vec![
            Arc::new(FakeEngine {
                id: AsrEngineId::OnnxParakeet,
                text: None,
            }),
            Arc::new(FakeEngine {
                id: AsrEngineId::WhisperCpp,
                text: Some("fallback worked"),
            }),
        ])
        .unwrap();
        assert_eq!(chain.transcribe(&[0.0]).unwrap().text, "fallback worked");
    }

    #[test]
    fn vad_trims_silence_without_erasing_speech() {
        let mut vad = SmoothedVad::new(Box::new(EnergyVad::new(4, 0.1)), 1, 1, 1);
        let pcm = [vec![0.0; 8], vec![0.8; 8], vec![0.0; 8]].concat();
        let trimmed = trim_with_vad(&mut vad, &pcm).unwrap();
        assert!(trimmed.len() < pcm.len());
        assert!(trimmed.iter().any(|sample| *sample > 0.5));
    }

    #[test]
    fn vad_falls_back_when_no_speech_is_detected() {
        let mut vad = EnergyVad::new(4, 0.5);
        let pcm = vec![0.01; 12];
        assert_eq!(trim_with_vad(&mut vad, &pcm).unwrap(), pcm);
    }
}
