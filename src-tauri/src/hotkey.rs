//! Hold-Fn → pill wiring for the demo shell.
//!
//! This installs an in-process CoreGraphics event tap that feeds Fn key-down /
//! key-up into the real [`whimpr_core`] dictation state machine, and turns the
//! machine's actions into `whimpr://flowbar/state` events the overlay pill
//! renders. There is no audio or ASR yet, so a finalized session is simulated as
//! completing shortly after key release — enough to see the full
//! recording → transcribing → done → idle loop driven by the actual state machine.
//!
//! In the shipping product this hook lives in a separate sidecar process (so heavy
//! inference can't stall it); running it in-process is an acceptable macOS-only
//! path for this demo and the early milestones.

/// Dictionary entry shape sent to the Hub UI (auto-learned entries flagged).
#[derive(Clone, serde::Serialize)]
pub struct DictEntryDto {
    pub correct: String,
    pub mishears: Vec<String>,
    pub auto: bool,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::DictEntryDto;
    use std::os::raw::c_void;
    use std::path::PathBuf;
    use std::ptr::{null, null_mut};
    use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Arc, Mutex, OnceLock, RwLock};
    use std::time::{Duration, Instant};

    use serde::Serialize;
    use tauri::{AppHandle, Emitter};
    use whimpr_core::state::{Action, BarState};
    use whimpr_core::{
        AsrEngine, CleanupContext, CleanupMode, CleanupProvider, Input, PipelineEvent, RecordMode,
        StateMachine, TriggerToken,
    };
    use whimpr_ipc::BindingId;

    const OVERLAY_LABEL: &str = "whimpr_bar";

    // --- CoreGraphics / CoreFoundation FFI (session event tap; may swallow PTT keys) ---
    type CFMachPortRef = *mut c_void;
    type CFRunLoopSourceRef = *mut c_void;
    type CFRunLoopRef = *mut c_void;
    type CFStringRef = *const c_void;
    type CFAllocatorRef = *const c_void;
    type CGEventRef = *mut c_void;
    type CGEventTapProxy = *mut c_void;
    type CGEventTapCallBack =
        extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventTapCreate(
            tap: u32,
            place: u32,
            options: u32,
            events_of_interest: u64,
            callback: CGEventTapCallBack,
            user_info: *mut c_void,
        ) -> CFMachPortRef;
        fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
        fn CGEventGetFlags(event: CGEventRef) -> u64;
        fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFMachPortCreateRunLoopSource(
            allocator: CFAllocatorRef,
            port: CFMachPortRef,
            order: isize,
        ) -> CFRunLoopSourceRef;
        fn CFRunLoopGetCurrent() -> CFRunLoopRef;
        fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
        fn CFRunLoopRun();
        static kCFRunLoopDefaultMode: CFStringRef;
    }

    const K_CG_SESSION_EVENT_TAP: u32 = 1;
    const K_CG_HEAD_INSERT: u32 = 0;
    /// Default tap — can suppress keys so Option+W doesn't type ∑.
    const K_CG_TAP_OPTION_DEFAULT: u32 = 0;
    const K_CG_EVENT_KEY_DOWN: u32 = 10;
    const K_CG_EVENT_KEY_UP: u32 = 11;
    const K_CG_EVENT_FLAGS_CHANGED: u32 = 12;
    const EVENTS_OF_INTEREST: u64 =
        (1 << K_CG_EVENT_KEY_DOWN) | (1 << K_CG_EVENT_KEY_UP) | (1 << K_CG_EVENT_FLAGS_CHANGED);
    const FLAG_SECONDARY_FN: u64 = 0x0080_0000;
    const K_CG_KEYBOARD_EVENT_KEYCODE: u32 = 9;
    const KEYCODE_FN: i64 = 63;
    const KEYCODE_ESC: i64 = 53;
    const K_CG_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
    const K_CG_TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;

    static PTT_HELD: AtomicBool = AtomicBool::new(false);
    static COMMAND_HELD: AtomicBool = AtomicBool::new(false);
    static COMMAND_FINISHING: AtomicBool = AtomicBool::new(false);
    static COMMAND_PROCESSING: AtomicBool = AtomicBool::new(false);
    static COMMAND_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    static BAR_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    /// Work kicked off by the CGEventTap — must never run on the tap callback
    /// thread or macOS disables the tap (Option+W then types ∑ into apps).
    static TAP_TX: OnceLock<Sender<TapCmd>> = OnceLock::new();

    enum TapCmd {
        PttDown { at_ms: u64 },
        PttUp { at_ms: u64 },
        CommandDown,
        CommandUp,
        Cancel,
    }

    static APP: OnceLock<AppHandle> = OnceLock::new();
    static MACHINE: OnceLock<Mutex<StateMachine>> = OnceLock::new();
    static CLOCK: OnceLock<Instant> = OnceLock::new();
    static FN_IS_DOWN: AtomicBool = AtomicBool::new(false);
    static TAP_PORT: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
    /// Bundle id of the app that was frontmost at record-start = the paste target.
    /// Cleanup uses it to format for the medium (email vs. text vs. chat).
    static TARGET_APP: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    /// ~200 chars of caret context captured at PTT-down for cleanup prompts.
    static WINDOW_CONTEXT: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    static CAPTURE: OnceLock<Mutex<Option<whimpr_audio::CaptureHandle>>> = OnceLock::new();
    static COMMAND_CAPTURE: OnceLock<Mutex<Option<whimpr_audio::CaptureHandle>>> = OnceLock::new();
    static ASR: OnceLock<RwLock<Option<Arc<dyn AsrEngine>>>> = OnceLock::new();
    static VAD: OnceLock<Arc<whimpr_asr::SileroVadTrimmer>> = OnceLock::new();
    static ASR_LOADING: AtomicBool = AtomicBool::new(false);
    /// Bumped to cancel in-flight live-preview ASR loops.
    static PREVIEW_GEN: AtomicU64 = AtomicU64::new(0);
    static PREVIEW_BUSY: AtomicBool = AtomicBool::new(false);
    static OPENAI: OnceLock<Mutex<Option<whimpr_cleanup::OpenAiProvider>>> = OnceLock::new();
    static OLLAMA: OnceLock<Mutex<Option<whimpr_cleanup::OpenAiProvider>>> = OnceLock::new();
    static ANTHROPIC: OnceLock<Mutex<Option<whimpr_cleanup::AnthropicProvider>>> = OnceLock::new();
    static LOCAL: OnceLock<Mutex<Option<crate::local_llm::LocalWorker>>> = OnceLock::new();
    static SETTINGS: OnceLock<Mutex<whimpr_core::Settings>> = OnceLock::new();
    static DICTIONARY: OnceLock<Mutex<whimpr_core::DictionaryStore>> = OnceLock::new();
    static SNIPPETS: OnceLock<Mutex<whimpr_core::SnippetStore>> = OnceLock::new();
    static TRANSFORMS: OnceLock<Mutex<whimpr_core::TransformStore>> = OnceLock::new();
    static STATS: OnceLock<Mutex<whimpr_core::StatsStore>> = OnceLock::new();

    struct CommandProcessingGuard;

    impl Drop for CommandProcessingGuard {
        fn drop(&mut self) {
            COMMAND_PROCESSING.store(false, Ordering::SeqCst);
        }
    }

    #[derive(Clone, Serialize)]
    struct BarPayload {
        state: &'static str,
        #[serde(skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    }

    #[derive(Clone, Serialize)]
    struct WavePayload {
        bars: Vec<f32>,
    }

    #[derive(Clone, Serialize)]
    struct PartialPayload {
        text: String,
    }

    /// The whisper ASR model to load: prefer the most accurate one present, in
    /// descending quality order, falling back to the small base model. Bigger
    /// English models mis-hear names/technical terms far less (and better ASR means
    /// less for cleanup and the dictionary to fix downstream).
    fn model_path() -> PathBuf {
        let dir = support_dir().join("models");
        for name in [
            "ggml-large-v3-turbo.bin",
            "ggml-medium.en.bin",
            "ggml-small.en.bin",
            "ggml-base.en.bin",
        ] {
            let p = dir.join(name);
            if p.exists() {
                return p;
            }
        }
        dir.join("ggml-base.en.bin")
    }

    fn support_dir() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join("Library/Application Support/WhimprFlow")
    }
    fn settings_path() -> PathBuf {
        support_dir().join("settings.json")
    }
    fn dict_path() -> PathBuf {
        support_dir().join("dictionary.json")
    }
    fn snippets_path() -> PathBuf {
        support_dir().join("snippets.json")
    }
    fn transforms_path() -> PathBuf {
        support_dir().join("transforms.json")
    }
    fn stats_path() -> PathBuf {
        support_dir().join("stats.json")
    }
    fn audio_dir() -> PathBuf {
        support_dir().join("audio")
    }

    fn unlink_audio(path: Option<&str>) {
        if let Some(path) = path {
            crate::audio_archive::delete_file(path);
        }
    }

    /// Seconds since the Unix epoch (UTC), or 0 if the clock is before the epoch.
    fn unix_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    /// Log one completed dictation to the stats store (words, speaking time, text,
    /// target app) and persist it. Powers both the Hub stats and the history list.
    pub fn record_dictation(
        raw: &str,
        cleaned: &str,
        duration_secs: f32,
        ts_unix: u64,
        audio_path: Option<String>,
    ) {
        let settings = current_settings();
        if !settings.store_history {
            unlink_audio(audio_path.as_deref());
            return;
        }
        let words = whimpr_core::stats::count_words(cleaned);
        if words == 0 {
            unlink_audio(audio_path.as_deref());
            return;
        }
        let app = TARGET_APP.get().and_then(|m| m.lock().unwrap().clone());
        if let Some(m) = STATS.get() {
            let mut store = m.lock().unwrap();
            let duration_ms = (duration_secs.max(0.0) * 1000.0) as u32;
            let chars = cleaned.chars().count() as u32;
            store.record(
                words,
                duration_ms,
                chars,
                ts_unix,
                cleaned.to_string(),
                raw.to_string(),
                app,
                audio_path,
            );
            let pruned = store.prune_older_than(settings.history_retention_days, unix_now());
            if !pruned.is_empty() {
                eprintln!("[whimpr] pruned {} expired history item(s)", pruned.len());
                for session in pruned {
                    unlink_audio(session.audio_path.as_deref());
                }
            }
            let _ = store.save(&stats_path());
        }
    }

    /// The most recent dictations for the Hub Home history list.
    pub fn history(limit: usize) -> Vec<whimpr_core::HistoryItem> {
        STATS
            .get()
            .map(|m| m.lock().unwrap().history(limit))
            .unwrap_or_default()
    }

    pub fn delete_history(ts_unix: u64) -> bool {
        let Some(m) = STATS.get() else {
            return false;
        };
        let mut store = m.lock().unwrap();
        if let Some(removed) = store.delete_at(ts_unix) {
            unlink_audio(removed.audio_path.as_deref());
            let _ = store.save(&stats_path());
            true
        } else {
            false
        }
    }

    pub fn clear_history() -> usize {
        let Some(m) = STATS.get() else {
            return 0;
        };
        let mut store = m.lock().unwrap();
        let removed = store.clear_all();
        let count = removed.len();
        for session in removed {
            unlink_audio(session.audio_path.as_deref());
        }
        if count > 0 {
            let _ = store.save(&stats_path());
        }
        count
    }

    pub fn prune_history() {
        let settings = current_settings();
        let Some(m) = STATS.get() else {
            return;
        };
        let mut store = m.lock().unwrap();
        let pruned = store.prune_older_than(settings.history_retention_days, unix_now());
        if !pruned.is_empty() {
            for session in pruned {
                unlink_audio(session.audio_path.as_deref());
            }
            let _ = store.save(&stats_path());
        }
    }

    pub fn history_audio(ts_unix: u64) -> Option<Vec<u8>> {
        let path = STATS
            .get()
            .and_then(|m| m.lock().unwrap().audio_path_for(ts_unix))?;
        crate::audio_archive::read_bytes(&path)
    }

    pub fn sessions_for_analysis(limit: usize) -> Vec<(String, Option<String>)> {
        STATS
            .get()
            .map(|m| {
                m.lock()
                    .unwrap()
                    .sessions
                    .iter()
                    .rev()
                    .filter(|s| !s.text.is_empty())
                    .take(limit)
                    .map(|s| (s.text.clone(), s.app.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The dictionary entries for the Hub Dictionary screen (auto-learned flagged).
    pub fn dictionary_entries() -> Vec<DictEntryDto> {
        DICTIONARY
            .get()
            .map(|m| {
                m.lock()
                    .unwrap()
                    .entries
                    .iter()
                    .map(|e| DictEntryDto {
                        correct: e.correct.clone(),
                        mishears: e.mishears.clone(),
                        auto: matches!(e.source, whimpr_core::DictSource::Auto),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Add a manual dictionary entry and persist.
    pub fn dictionary_add(correct: String, mishears: Vec<String>) {
        if let Some(m) = DICTIONARY.get() {
            let mut store = m.lock().unwrap();
            store.add(correct, mishears, whimpr_core::DictSource::Manual);
            let _ = store.save(&dict_path());
        }
    }

    /// Remove a dictionary entry by spelling and persist.
    pub fn dictionary_remove(correct: &str) {
        if let Some(m) = DICTIONARY.get() {
            let mut store = m.lock().unwrap();
            if store.remove(correct) {
                let _ = store.save(&dict_path());
            }
        }
    }

    /// Add an AUTO-learned entry (from the post-paste correction observer) and persist.
    /// Marked ✨ auto in the UI. No-op if it would duplicate an existing entry's data.
    pub fn dictionary_learn(correct: String, mishears: Vec<String>) {
        if let Some(m) = DICTIONARY.get() {
            let mut store = m.lock().unwrap();
            store.add(correct, mishears, whimpr_core::DictSource::Auto);
            let _ = store.save(&dict_path());
        }
    }

    pub fn snippets_list() -> Vec<whimpr_core::Snippet> {
        SNIPPETS
            .get()
            .map(|m| m.lock().unwrap().snippets.clone())
            .unwrap_or_default()
    }

    pub fn snippet_add(trigger: String, expansion: String) {
        if let Some(m) = SNIPPETS.get() {
            let mut store = m.lock().unwrap();
            store.add(trigger, expansion);
            let _ = store.save(&snippets_path());
        }
    }

    pub fn snippet_remove(trigger: &str) -> bool {
        let Some(m) = SNIPPETS.get() else {
            return false;
        };
        let mut store = m.lock().unwrap();
        let ok = store.remove(trigger);
        if ok {
            let _ = store.save(&snippets_path());
        }
        ok
    }

    pub fn transforms_list() -> Vec<whimpr_core::TransformPreset> {
        TRANSFORMS
            .get()
            .map(|m| m.lock().unwrap().presets.clone())
            .unwrap_or_default()
    }

    pub fn transform_upsert(preset: whimpr_core::TransformPreset) {
        if let Some(m) = TRANSFORMS.get() {
            let mut store = m.lock().unwrap();
            store.upsert(preset);
            let _ = store.save(&transforms_path());
        }
    }

    pub fn transform_remove(id: &str) -> bool {
        let Some(m) = TRANSFORMS.get() else {
            return false;
        };
        let mut store = m.lock().unwrap();
        let ok = store.remove(id);
        if ok {
            let _ = store.save(&transforms_path());
        }
        ok
    }

    pub fn language_stats(limit: usize) -> whimpr_core::LanguageStats {
        STATS
            .get()
            .map(|m| m.lock().unwrap().language_stats(limit))
            .unwrap_or_default()
    }

    /// Aggregated stats for the Hub. `tz_offset_minutes` is the UI's
    /// `Date.getTimezoneOffset()` so day math matches the user's local clock.
    pub fn stats_summary(tz_offset_minutes: i32) -> whimpr_core::StatsSummary {
        STATS
            .get()
            .map(|m| m.lock().unwrap().summary(tz_offset_minutes, unix_now()))
            .unwrap_or_else(|| {
                whimpr_core::StatsStore::default().summary(tz_offset_minutes, unix_now())
            })
    }

    /// Read an API key from an env var or the OS keychain (never a plaintext file).
    fn read_key(account: &str, env_var: &str) -> Option<String> {
        if let Ok(k) = std::env::var(env_var) {
            let k = k.trim().to_string();
            if !k.is_empty() {
                return Some(k);
            }
        }
        keyring::Entry::new("com.whimpr.whimprflow", account)
            .ok()
            .and_then(|e| e.get_password().ok())
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
    }
    fn read_openai_key() -> Option<String> {
        read_key("openai_api_key", "OPENAI_API_KEY")
    }
    fn read_anthropic_key() -> Option<String> {
        read_key("anthropic_api_key", "ANTHROPIC_API_KEY")
    }

    /// A snapshot of the current settings.
    pub fn current_settings() -> whimpr_core::Settings {
        SETTINGS
            .get()
            .map(|m| m.lock().unwrap().clone())
            .unwrap_or_default()
    }
    /// Apply new settings and rebuild the cloud providers (picks up model changes).
    pub fn update_settings(new: whimpr_core::Settings) {
        if let Some(m) = SETTINGS.get() {
            *m.lock().unwrap() = new.clone();
        }
        let _ = new.save(&settings_path());
        if let Some(engine) = ASR
            .get()
            .and_then(|slot| slot.read().ok()?.as_ref().cloned())
        {
            if let Err(error) = engine.set_language(&new.dictation_language) {
                eprintln!("[whimpr] ASR language update failed: {error}");
            }
        }
        prune_history();
        rebuild_providers();
    }

    /// (Re)build the cloud cleanup providers from the current keys + settings. Called
    /// at startup and whenever a key or model changes, so edits take effect live.
    pub fn rebuild_providers() {
        let settings = current_settings();
        let openai = read_openai_key().map(|k| {
            whimpr_cleanup::OpenAiProvider::with_base_url(
                k,
                settings.openai_model.clone(),
                Some(settings.openai_base_url.clone()),
            )
        });
        let ollama_base = if settings.ollama_base_url.trim().is_empty() {
            "http://localhost:11434/v1".to_string()
        } else {
            settings.ollama_base_url.clone()
        };
        let ollama = Some(whimpr_cleanup::OpenAiProvider::with_base_url(
            String::new(),
            settings.ollama_model.clone(),
            Some(ollama_base),
        ));
        let anthropic = read_anthropic_key()
            .map(|k| whimpr_cleanup::AnthropicProvider::new(k, settings.anthropic_model.clone()));
        eprintln!(
            "[whimpr] cleanup providers: openai={}, ollama={}, anthropic={}",
            openai.is_some(),
            ollama.is_some(),
            anthropic.is_some()
        );
        match OPENAI.get() {
            Some(m) => *m.lock().unwrap() = openai,
            None => {
                let _ = OPENAI.set(Mutex::new(openai));
            }
        }
        match OLLAMA.get() {
            Some(m) => *m.lock().unwrap() = ollama,
            None => {
                let _ = OLLAMA.set(Mutex::new(ollama));
            }
        }
        match ANTHROPIC.get() {
            Some(m) => *m.lock().unwrap() = anthropic,
            None => {
                let _ = ANTHROPIC.set(Mutex::new(anthropic));
            }
        }
    }

    pub fn reload_local_worker() {
        std::thread::spawn(|| {
            let worker = crate::local_llm::spawn_default();
            let slot = LOCAL.get_or_init(|| Mutex::new(None));
            *slot.lock().unwrap() = worker;
        });
    }

    /// Clean a raw transcript per the current settings (mode + level), feeding in the
    /// dictionary vocabulary relevant to this utterance. Falls back to raw whenever
    /// cleanup is off, the provider is unavailable, it errors, or the gates reject it.
    fn clean_transcript(raw: &str) -> String {
        let settings = current_settings();
        let level = settings.cleanup_level;
        if matches!(settings.cleanup_mode, CleanupMode::Raw) || level.bypasses_llm() {
            return raw.to_string();
        }
        // Turn explicit spoken layout cues ("new line", "new paragraph") into break
        // markers up front — the model passes an opaque marker through reliably but
        // mangles the literal cue words. The model sees `raw` (with markers); the gate
        // and any raw fallback use `raw_out` (markers restored to real breaks) so we
        // never paste a "[[NL]]" token or lose an explicit break.
        let raw_norm = whimpr_core::cleanup::pre_normalize_layout(raw);
        let raw = raw_norm.as_str();
        let raw_out = whimpr_core::cleanup::post_process(&raw_norm);
        let vocab = DICTIONARY
            .get()
            .map(|d| d.lock().unwrap().prefilter(raw, 15))
            .unwrap_or_default();
        let app_bundle_id = TARGET_APP.get().and_then(|m| m.lock().unwrap().clone());
        if let Some(app) = app_bundle_id.as_deref() {
            eprintln!("[whimpr] cleanup target app: {app}");
        }
        let window_context = if settings.context_awareness {
            WINDOW_CONTEXT
                .get()
                .and_then(|m| m.lock().unwrap().clone())
        } else {
            None
        };
        if let Some(ctx) = window_context.as_deref() {
            eprintln!(
                "[whimpr] cleanup window context ({} chars)",
                ctx.chars().count()
            );
        }
        let ctx = CleanupContext {
            level,
            vocab,
            app_bundle_id,
            writing_style: settings.writing_style,
            window_context,
        };
        // Run the on-device model with the same prompt + per-app formatting.
        let run_local = || -> Option<anyhow::Result<String>> {
            let messages = whimpr_core::cleanup::build_messages(raw, &ctx);
            let local = LOCAL.get_or_init(|| Mutex::new(crate::local_llm::spawn_default()));
            let mut worker = local.lock().unwrap();
            if worker.is_none() {
                *worker = crate::local_llm::spawn_default();
            }
            let first = worker.as_mut()?.cleanup(&messages);
            match first {
                Ok(cleaned) => Some(Ok(cleaned)),
                Err(first_error) => {
                    eprintln!("[whimpr] local worker failed; restarting once: {first_error}");
                    *worker = crate::local_llm::spawn_default();
                    Some(match worker.as_mut() {
                        Some(restarted) => restarted.cleanup(&messages),
                        None => Err(first_error),
                    })
                }
            }
        };
        // Selected provider, falling back to local when a cloud key can't be read
        // (so cleanup still runs) — and Local mode uses the worker directly.
        let result: Option<anyhow::Result<String>> = match settings.cleanup_mode {
            CleanupMode::OpenAi => OPENAI
                .get()
                .and_then(|m| m.lock().unwrap().as_ref().map(|p| p.cleanup(raw, &ctx)))
                .or_else(run_local),
            CleanupMode::Ollama => OLLAMA
                .get()
                .and_then(|m| m.lock().unwrap().as_ref().map(|p| p.cleanup(raw, &ctx)))
                .or_else(run_local),
            CleanupMode::Anthropic => ANTHROPIC
                .get()
                .and_then(|m| m.lock().unwrap().as_ref().map(|p| p.cleanup(raw, &ctx)))
                .or_else(run_local),
            CleanupMode::Local => run_local(),
            CleanupMode::Raw => None,
        };
        match result {
            Some(Ok(cleaned)) => {
                // Deterministic safety net: convert any leftover spoken layout cue the
                // model missed into real line breaks, strip stray code fences, cap blank
                // lines. Guarantees no "new line"/"new paragraph" word reaches the cursor.
                let cleaned = whimpr_core::cleanup::post_process(&cleaned);
                if whimpr_core::cleanup::evaluate_gates(&raw_out, &cleaned, level).passed() {
                    cleaned
                } else {
                    eprintln!("[whimpr] cleanup gate rejected the edit — pasting raw");
                    raw_out
                }
            }
            Some(Err(e)) => {
                eprintln!("[whimpr] cleanup failed ({e}) — pasting raw");
                raw_out
            }
            None => {
                if matches!(settings.cleanup_mode, CleanupMode::Local) {
                    eprintln!("[whimpr] local cleanup worker unavailable — pasting raw");
                } else if matches!(settings.cleanup_mode, CleanupMode::Ollama) {
                    eprintln!(
                        "[whimpr] Ollama unavailable — is it running? (`ollama serve`) — pasting raw"
                    );
                } else {
                    eprintln!("[whimpr] cleanup provider has no API key — pasting raw");
                }
                raw_out
            }
        }
    }

    fn now_ms() -> u64 {
        CLOCK
            .get()
            .map(|c| c.elapsed().as_millis() as u64)
            .unwrap_or(0)
    }

    fn bar_name(b: BarState) -> &'static str {
        match b {
            BarState::Idle => "idle",
            // Show the waveform shell immediately; mic bars fill in once capture starts.
            BarState::Recording => "recording",
            BarState::Locked => "locked",
            BarState::Transcribing => "transcribing",
            BarState::Done => "done",
            BarState::Cancelled => "cancelled",
            BarState::Error => "error",
        }
    }

    /// Whisper often invents short "stage direction" lines on near-silence
    /// (`*sad music*`, `[Silence]`, etc.). Reject those so we don't paste junk.
    fn is_whisper_hallucination(text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return true;
        }
        // Asterisk / bracket stage directions: "*sad music*", "[Silence]".
        if (trimmed.starts_with('*') && trimmed.ends_with('*') && trimmed.len() < 48)
            || (trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() < 48)
        {
            return true;
        }
        let normalized = trimmed
            .trim_matches(|c: char| {
                matches!(
                    c,
                    '*' | '"'
                        | '\''
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '['
                        | ']'
                        | '('
                        | ')'
                        | '.'
                        | '!'
                        | '?'
                )
            })
            .trim()
            .to_ascii_lowercase();
        matches!(
            normalized.as_str(),
            "silence"
                | "blank audio"
                | "no audio"
                | "inaudible"
                | "music"
                | "sad music"
                | "applause"
                | "laughing"
                | "laughter"
                | "coughing"
                | "cough"
                | "..."
                | "…"
        ) || (normalized.starts_with("music") && normalized.len() < 24)
    }

    fn emit_bar(app: &AppHandle, state: &'static str) {
        emit_bar_msg(app, state, None);
    }

    fn emit_bar_msg(app: &AppHandle, state: &'static str, message: Option<String>) {
        BAR_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        eprintln!("[whimpr] pill -> {state}");
        crate::overlay::present(app);
        crate::overlay::dispatch_state(app, state, message.clone());
        let _ = app.emit_to(
            OVERLAY_LABEL,
            "whimpr://flowbar/state",
            BarPayload { state, message },
        );
    }

    fn emit_partial(app: &AppHandle, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        crate::overlay::dispatch_partial(app, trimmed);
        let _ = app.emit_to(
            OVERLAY_LABEL,
            "whimpr://flowbar/partial",
            PartialPayload {
                text: trimmed.to_string(),
            },
        );
    }

    fn capture_window_context_async() {
        if !current_settings().context_awareness {
            *WINDOW_CONTEXT.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
            return;
        }
        *WINDOW_CONTEXT.get_or_init(|| Mutex::new(None)).lock().unwrap() = None;
        std::thread::spawn(|| {
            let ctx = crate::context::read_caret_context(100, 100);
            if let Some(ref text) = ctx {
                eprintln!(
                    "[whimpr] caret context captured ({} chars)",
                    text.chars().count()
                );
            }
            *WINDOW_CONTEXT.get_or_init(|| Mutex::new(None)).lock().unwrap() = ctx;
        });
    }

    fn start_live_preview(app: AppHandle, session: whimpr_core::SessionId) {
        if !current_settings().live_preview_asr {
            eprintln!("[whimpr] live preview off (Settings → Experimental)");
            return;
        }
        let gen = PREVIEW_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        eprintln!("[whimpr] live preview started (gen {gen})");
        std::thread::spawn(move || {
            // Wait briefly so the mic buffer has something to score.
            std::thread::sleep(Duration::from_millis(650));
            loop {
                if PREVIEW_GEN.load(Ordering::SeqCst) != gen {
                    break;
                }
                let still_recording = MACHINE
                    .get()
                    .map(|machine| {
                        matches!(
                            machine.lock().unwrap().state(),
                            whimpr_core::DictationState::Recording {
                                session: current,
                                ..
                            } if current == session
                        )
                    })
                    .unwrap_or(false);
                if !still_recording {
                    break;
                }
                if PREVIEW_BUSY.swap(true, Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(250));
                    continue;
                }
                let snapshot = CAPTURE
                    .get()
                    .and_then(|slot| slot.lock().ok()?.as_ref()?.snapshot());
                let Some((samples, sample_rate)) = snapshot else {
                    PREVIEW_BUSY.store(false, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(300));
                    continue;
                };
                let pcm = whimpr_audio::resample_to_16k(&samples, sample_rate);
                // Trailing ~4s keeps preview latency bounded on long holds.
                const PREVIEW_SAMPLES: usize = 16_000 * 4;
                let min_samples = 12_000; // ~0.75s
                if pcm.len() < min_samples {
                    PREVIEW_BUSY.store(false, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(300));
                    continue;
                }
                let start = pcm.len().saturating_sub(PREVIEW_SAMPLES);
                let chunk = &pcm[start..];
                let peak = chunk.iter().fold(0f32, |m, &s| m.max(s.abs()));
                if peak < 0.01 {
                    PREVIEW_BUSY.store(false, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(300));
                    continue;
                }
                let asr = ASR
                    .get()
                    .and_then(|slot| slot.read().ok()?.as_ref().cloned());
                let Some(asr) = asr else {
                    // Model still loading — keep trying instead of giving up.
                    PREVIEW_BUSY.store(false, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                };
                let result = asr.transcribe(chunk);
                PREVIEW_BUSY.store(false, Ordering::SeqCst);
                if PREVIEW_GEN.load(Ordering::SeqCst) != gen {
                    break;
                }
                match result {
                    Ok(t) => {
                        let text = t.text.trim().to_string();
                        if !text.is_empty() && !is_whisper_hallucination(&text) {
                            eprintln!("[whimpr] preview: \"{text}\"");
                            emit_partial(&app, &text);
                        }
                    }
                    Err(error) => {
                        eprintln!("[whimpr] preview ASR skipped: {error}");
                    }
                }
                std::thread::sleep(Duration::from_millis(450));
            }
        });
    }

    fn fail_pipeline(app: &AppHandle, session: whimpr_core::SessionId, message: &'static str) {
        emit_bar_msg(app, "error", Some(message.into()));
        let sequence = BAR_SEQUENCE.load(Ordering::SeqCst);
        handle_input(Input::Pipeline(PipelineEvent::Failed { session }));
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(2_500));
            if BAR_SEQUENCE.load(Ordering::SeqCst) == sequence {
                emit_bar(&app, "idle");
            }
        });
    }

    fn emit_command_error(app: &AppHandle, message: impl Into<String>) {
        emit_bar_msg(app, "error", Some(message.into()));
        let sequence = BAR_SEQUENCE.load(Ordering::SeqCst);
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(2_500));
            if BAR_SEQUENCE.load(Ordering::SeqCst) == sequence {
                emit_bar(&app, "idle");
            }
        });
    }

    /// Feed one input into the shared state machine and enact its actions.
    fn handle_input(input: Input) {
        let (Some(app), Some(machine)) = (APP.get(), MACHINE.get()) else {
            return;
        };
        let actions = {
            let mut m = machine.lock().unwrap();
            m.step(input)
        };
        for action in actions {
            apply_action(app, action);
        }
    }

    fn apply_action(app: &AppHandle, action: Action) {
        match action {
            Action::ShowBar(bar) => {
                emit_bar(app, bar_name(bar));
                // Let the "done" tick linger briefly before returning to idle.
                if bar == BarState::Done {
                    let sequence = BAR_SEQUENCE.load(Ordering::SeqCst);
                    let app2 = app.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(500));
                        if BAR_SEQUENCE.load(Ordering::SeqCst) == sequence {
                            emit_bar(&app2, "idle");
                        }
                    });
                }
            }
            // Start the microphone; stream real RMS bars to the pill waveform.
            // Runs off the tap thread so the mic-permission prompt can't stall keys.
            Action::StartCapture { session, mode } => {
                if current_settings().pause_media_while_dictating {
                    let _ = app.run_on_main_thread(|| {
                        crate::media::on_dictation_start();
                    });
                }
                let app_thread = app.clone();
                std::thread::spawn(move || {
                    let app_cb = app_thread.clone();
                    match whimpr_audio::start(move |bars| {
                        crate::overlay::dispatch_waveform(&app_cb, bars);
                        let _ = app_cb.emit_to(
                            OVERLAY_LABEL,
                            "whimpr://audio/waveform",
                            WavePayload {
                                bars: bars.to_vec(),
                            },
                        );
                    }) {
                        Ok(handle) => {
                            eprintln!("[whimpr] mic capture started — waveform active");
                            *CAPTURE.get_or_init(|| Mutex::new(None)).lock().unwrap() =
                                Some(handle);
                            std::thread::sleep(Duration::from_millis(160));
                            let still_recording = MACHINE
                                .get()
                                .map(|machine| {
                                    matches!(
                                        machine.lock().unwrap().state(),
                                        whimpr_core::DictationState::Recording {
                                            session: current,
                                            ..
                                        } if current == session
                                    )
                                })
                                .unwrap_or(false);
                            if still_recording {
                                emit_bar(
                                    &app_thread,
                                    if mode == RecordMode::Locked {
                                        "locked"
                                    } else {
                                        "recording"
                                    },
                                );
                                start_live_preview(app_thread.clone(), session);
                            }
                        }
                        Err(e) => {
                            eprintln!("[whimpr] mic capture failed to start: {e}");
                            emit_bar_msg(
                                &app_thread,
                                "error",
                                Some("No microphone detected".into()),
                            );
                        }
                    }
                });
            }
            // Stop the mic, transcribe the buffered audio, and advance the machine.
            Action::StopCaptureAndFinalize { session } => {
                PREVIEW_GEN.fetch_add(1, Ordering::SeqCst);
                if current_settings().sound_on_start {
                    crate::sound::play_stop_ping();
                }
                if current_settings().pause_media_while_dictating {
                    let _ = app.run_on_main_thread(|| {
                        crate::media::on_dictation_stop();
                    });
                }
                let app2 = app.clone();
                let handle = CAPTURE.get().and_then(|slot| slot.lock().unwrap().take());
                std::thread::spawn(move || {
                    // Let a mid-flight preview finish so it releases the ASR lock.
                    for _ in 0..40 {
                        if !PREVIEW_BUSY.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    let Some(res) = handle.and_then(|h| h.stop()) else {
                        eprintln!("[whimpr] no audio captured");
                        fail_pipeline(&app2, session, "Microphone unavailable");
                        return;
                    };
                    let peak = res.samples.iter().fold(0f32, |m, &s| m.max(s.abs()));
                    eprintln!(
                        "[whimpr] captured {} samples @ {} Hz (~{:.2}s), peak {:.4}",
                        res.samples.len(),
                        res.sample_rate,
                        res.duration_secs(),
                        peak
                    );
                    if peak < 0.005 {
                        eprintln!(
                            "[whimpr] ⚠ audio is silent — the mic isn't being captured. Grant \
                             Microphone access to your terminal (System Settings → Privacy & \
                             Security → Microphone), then fully quit + reopen it and rerun."
                        );
                        fail_pipeline(&app2, session, "We couldn't hear you");
                        return;
                    }
                    let Some(asr) = ASR
                        .get()
                        .and_then(|slot| slot.read().ok()?.as_ref().cloned())
                    else {
                        eprintln!("[whimpr] ASR not ready (model still loading or missing)");
                        fail_pipeline(&app2, session, "No Model Available");
                        return;
                    };
                    let pcm = whimpr_audio::resample_to_16k(&res.samples, res.sample_rate);
                    let pcm = if let Some(vad) = VAD.get() {
                        match vad.trim(&pcm) {
                            Ok(trimmed) => {
                                eprintln!(
                                    "[whimpr] VAD kept {:.0}% of captured audio",
                                    trimmed.len() as f64 / pcm.len().max(1) as f64 * 100.0
                                );
                                trimmed
                            }
                            Err(error) => {
                                eprintln!("[whimpr] VAD failed, using full recording: {error}");
                                pcm
                            }
                        }
                    } else {
                        pcm
                    };
                    let settings = current_settings();
                    let ts_unix = unix_now();
                    let audio_path = if settings.store_history && settings.retain_audio && !pcm.is_empty()
                    {
                        let path = audio_dir().join(format!("{ts_unix}.wav"));
                        match crate::audio_archive::write_wav(&path, &pcm) {
                            Ok(p) => {
                                eprintln!("[whimpr] retained audio {}", p.display());
                                Some(p.to_string_lossy().into_owned())
                            }
                            Err(e) => {
                                eprintln!("[whimpr] audio retain failed: {e}");
                                None
                            }
                        }
                    } else {
                        None
                    };
                    match asr.transcribe(&pcm) {
                        Ok(t) => {
                            let mut raw = t.text;
                            if let Some(m) = SNIPPETS.get() {
                                if let Some(expanded) = m.lock().unwrap().expand(&raw) {
                                    eprintln!("[whimpr] snippet trigger matched — expanding");
                                    raw = expanded;
                                }
                            }
                            eprintln!("[whimpr] TRANSCRIPT: \"{}\"", raw);
                            if raw.trim().is_empty() {
                                unlink_audio(audio_path.as_deref());
                                fail_pipeline(&app2, session, "We couldn't hear you");
                                return;
                            }
                            if is_whisper_hallucination(&raw) {
                                eprintln!(
                                    "[whimpr] dropping silence hallucination: \"{}\"",
                                    raw.trim()
                                );
                                unlink_audio(audio_path.as_deref());
                                fail_pipeline(&app2, session, "We couldn't hear you");
                                return;
                            }
                            // Clean the transcript (cloud LLM if configured), then paste.
                            // Keep the pill on a single clear processing state.
                            emit_bar_msg(&app2, "transcribing", Some("Transcribing…".into()));
                            let text = clean_transcript(&raw);
                            if text != raw {
                                eprintln!("[whimpr] CLEANED:   \"{}\"", text);
                            }
                            if !text.is_empty() {
                                emit_bar_msg(&app2, "transcribing", Some("Transcribing…".into()));
                                if let Err(e) = crate::paste::paste_text(&text) {
                                    eprintln!("[whimpr] paste failed: {e}");
                                    unlink_audio(audio_path.as_deref());
                                    let message = if e.to_string().contains("secure text fields") {
                                        "Dictation is disabled in password fields"
                                    } else {
                                        "Couldn't insert text — check Accessibility"
                                    };
                                    fail_pipeline(&app2, session, message);
                                    return;
                                } else {
                                    if current_settings().sound_on_start {
                                        crate::sound::play_done_ping();
                                    }
                                    record_dictation(
                                        &raw,
                                        &text,
                                        res.duration_secs(),
                                        ts_unix,
                                        audio_path,
                                    );
                                    crate::autolearn::watch_correction(&text);
                                }
                            } else {
                                unlink_audio(audio_path.as_deref());
                            }
                            handle_input(Input::Pipeline(PipelineEvent::Committed { session }));
                        }
                        Err(e) => {
                            eprintln!("[whimpr] ASR error: {e}");
                            unlink_audio(audio_path.as_deref());
                            fail_pipeline(&app2, session, "Something's not right");
                        }
                    }
                });
            }
            Action::DiscardCapture { .. } => {
                PREVIEW_GEN.fetch_add(1, Ordering::SeqCst);
                if current_settings().pause_media_while_dictating {
                    let _ = app.run_on_main_thread(|| {
                        crate::media::on_dictation_stop();
                    });
                }
                if let Some(slot) = CAPTURE.get() {
                    if let Some(handle) = slot.lock().unwrap().take() {
                        let _ = handle.stop();
                    }
                }
            }
            Action::PlayPing => {
                if current_settings().sound_on_start {
                    crate::sound::play_record_ping();
                }
            }
            // The ASR path (StopCaptureAndFinalize) now drives pipeline completion.
            Action::RunPipeline { .. } => {}
            // WarnSessionCap: no-op for now.
            _ => {}
        }
    }

    fn current_binding() -> whimpr_core::HotkeyBinding {
        let raw = current_settings().ptt_hotkey;
        whimpr_core::parse_hotkey(&raw).unwrap_or_default()
    }

    fn ptt_down(at_ms: u64) {
        if PTT_HELD.swap(true, Ordering::SeqCst) {
            return;
        }
        ptt_down_work(at_ms);
    }

    fn ptt_down_work(at_ms: u64) {
        eprintln!("[whimpr] PTT DOWN");
        let target = crate::appctx::frontmost_bundle_id();
        *TARGET_APP.get_or_init(|| Mutex::new(None)).lock().unwrap() = target;
        capture_window_context_async();
        handle_input(Input::Trigger(TriggerToken::Down {
            binding: BindingId::PushToTalk,
            at_ms,
        }));
    }

    fn ptt_up(at_ms: u64) {
        if !PTT_HELD.swap(false, Ordering::SeqCst) {
            return;
        }
        ptt_up_work(at_ms);
    }

    fn ptt_up_work(at_ms: u64) {
        eprintln!("[whimpr] PTT UP");
        handle_input(Input::Trigger(TriggerToken::Up {
            binding: BindingId::PushToTalk,
            at_ms,
        }));
    }

    fn enqueue_tap(cmd: TapCmd) {
        if let Some(tx) = TAP_TX.get() {
            let _ = tx.send(cmd);
        }
    }

    fn start_tap_worker() {
        let (tx, rx) = mpsc::channel::<TapCmd>();
        let _ = TAP_TX.set(tx);
        std::thread::spawn(move || {
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    TapCmd::PttDown { at_ms } => ptt_down_work(at_ms),
                    TapCmd::PttUp { at_ms } => ptt_up_work(at_ms),
                    TapCmd::CommandDown => command_down(),
                    TapCmd::CommandUp => command_up(),
                    TapCmd::Cancel => {
                        if COMMAND_HELD.load(Ordering::SeqCst)
                            || COMMAND_FINISHING.load(Ordering::SeqCst)
                            || COMMAND_PROCESSING.load(Ordering::SeqCst)
                        {
                            command_cancel();
                        }
                        if PTT_HELD.swap(false, Ordering::SeqCst) {
                            handle_input(Input::Trigger(TriggerToken::Cancel {
                                at_ms: now_ms(),
                            }));
                        }
                    }
                }
            }
        });
    }

    fn command_down() {
        if !current_settings().command_mode_enabled {
            return;
        }
        if PTT_HELD.load(Ordering::SeqCst)
            || COMMAND_PROCESSING.load(Ordering::SeqCst)
            || COMMAND_HELD.swap(true, Ordering::SeqCst)
        {
            return;
        }
        COMMAND_FINISHING.store(false, Ordering::SeqCst);
        COMMAND_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        eprintln!("[whimpr] COMMAND MODE DOWN");
        *TARGET_APP.get_or_init(|| Mutex::new(None)).lock().unwrap() =
            crate::appctx::frontmost_bundle_id();
        capture_window_context_async();
        emit_bar(APP.get().expect("app installed"), "listening");
        let app = APP.get().expect("app installed").clone();
        std::thread::spawn(move || {
            if current_settings().sound_on_start {
                crate::sound::play_record_ping();
            }
            let app_cb = app.clone();
            match whimpr_audio::start(move |bars| {
                crate::overlay::dispatch_waveform(&app_cb, bars);
            }) {
                Ok(handle) => {
                    if COMMAND_HELD.load(Ordering::SeqCst)
                        || COMMAND_FINISHING.load(Ordering::SeqCst)
                    {
                        *COMMAND_CAPTURE
                            .get_or_init(|| Mutex::new(None))
                            .lock()
                            .unwrap() = Some(handle);
                        emit_bar(&app, "recording");
                    } else {
                        let _ = handle.stop();
                    }
                }
                Err(error) => {
                    COMMAND_HELD.store(false, Ordering::SeqCst);
                    emit_bar_msg(
                        &app,
                        "error",
                        Some(format!("Microphone unavailable: {error}")),
                    );
                }
            }
        });
    }

    fn command_up() {
        if !COMMAND_HELD.swap(false, Ordering::SeqCst) {
            return;
        }
        COMMAND_FINISHING.store(true, Ordering::SeqCst);
        eprintln!("[whimpr] COMMAND MODE UP");
        let app = APP.get().expect("app installed").clone();
        let command_sequence = COMMAND_SEQUENCE.load(Ordering::SeqCst);
        std::thread::spawn(move || {
            COMMAND_PROCESSING.store(true, Ordering::SeqCst);
            let _processing_guard = CommandProcessingGuard;
            let mut handle = None;
            for _ in 0..50 {
                handle = COMMAND_CAPTURE
                    .get_or_init(|| Mutex::new(None))
                    .lock()
                    .unwrap()
                    .take();
                if handle.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            COMMAND_FINISHING.store(false, Ordering::SeqCst);
            if COMMAND_SEQUENCE.load(Ordering::SeqCst) != command_sequence {
                if let Some(capture) = handle {
                    let _ = capture.stop();
                }
                return;
            }
            let Some(audio) = handle.and_then(|capture| capture.stop()) else {
                emit_command_error(&app, "Command Mode captured no audio");
                return;
            };
            emit_bar_msg(&app, "transcribing", Some("Transcribing command…".into()));
            let Some(asr) = ASR
                .get()
                .and_then(|slot| slot.read().ok()?.as_ref().cloned())
            else {
                emit_command_error(&app, "No Model Available");
                return;
            };
            let pcm = whimpr_audio::resample_to_16k(&audio.samples, audio.sample_rate);
            let instruction = match asr.transcribe(&pcm) {
                Ok(transcript) if !transcript.text.trim().is_empty() => transcript.text,
                Ok(_) => {
                    emit_command_error(&app, "We couldn't hear your command");
                    return;
                }
                Err(error) => {
                    eprintln!("[whimpr] command ASR failed: {error}");
                    emit_command_error(&app, "Command transcription failed");
                    return;
                }
            };
            let selected = match crate::paste::copy_selection() {
                Ok(selection) => selection.unwrap_or_default(),
                Err(error) => {
                    eprintln!("[whimpr] command selection read failed: {error}");
                    String::new()
                }
            };
            if selected.split_whitespace().count() > 1_000 {
                emit_command_error(
                    &app,
                    "Too long to transform — select fewer than 1,000 words",
                );
                return;
            }
            let settings = current_settings();
            emit_bar_msg(&app, "formatting", Some("Applying command…".into()));
            match crate::transforms::apply_via_ollama(
                &selected,
                &instruction,
                &settings.ollama_base_url,
                &settings.ollama_model,
            ) {
                Ok(output) => {
                    if COMMAND_SEQUENCE.load(Ordering::SeqCst) != command_sequence {
                        return;
                    }
                    if let Err(error) = crate::paste::paste_text(&output) {
                        eprintln!("[whimpr] command paste failed: {error}");
                        emit_command_error(&app, "Command Mode couldn't update the text");
                        return;
                    }
                    emit_bar(&app, "done");
                    let sequence = BAR_SEQUENCE.load(Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(700));
                    if BAR_SEQUENCE.load(Ordering::SeqCst) == sequence {
                        emit_bar(&app, "idle");
                    }
                }
                Err(error) => {
                    eprintln!("[whimpr] command transform failed: {error}");
                    emit_command_error(&app, "Command Mode needs Ollama running");
                }
            }
        });
    }

    fn command_cancel() {
        if !COMMAND_HELD.swap(false, Ordering::SeqCst)
            && !COMMAND_FINISHING.swap(false, Ordering::SeqCst)
            && !COMMAND_PROCESSING.load(Ordering::SeqCst)
        {
            return;
        }
        COMMAND_SEQUENCE.fetch_add(1, Ordering::SeqCst);
        if let Some(capture) = COMMAND_CAPTURE
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap()
            .take()
        {
            let _ = capture.stop();
        }
        if let Some(app) = APP.get() {
            emit_bar(app, "cancelled");
        }
    }

    /// Returning null from the tap callback drops the event so it never reaches apps.
    fn swallow_event() -> CGEventRef {
        null_mut()
    }

    extern "C" fn tap_callback(
        _proxy: CGEventTapProxy,
        etype: u32,
        event: CGEventRef,
        _info: *mut c_void,
    ) -> CGEventRef {
        if etype == K_CG_TAP_DISABLED_BY_TIMEOUT || etype == K_CG_TAP_DISABLED_BY_USER_INPUT {
            let port = TAP_PORT.load(Ordering::SeqCst);
            if !port.is_null() {
                eprintln!("[whimpr] event tap was disabled — re-enabling (prevents ∑ leak)");
                unsafe { CGEventTapEnable(port, true) };
            }
            return event;
        }

        let binding = current_binding();
        let at_ms = now_ms();
        let keycode = unsafe { CGEventGetIntegerValueField(event, K_CG_KEYBOARD_EVENT_KEYCODE) };
        let flags = unsafe { CGEventGetFlags(event) };

        // Esc cancels from any state (let Esc through so other apps still see it).
        if etype == K_CG_EVENT_KEY_DOWN && keycode == KEYCODE_ESC {
            let busy = COMMAND_HELD.load(Ordering::SeqCst)
                || COMMAND_FINISHING.load(Ordering::SeqCst)
                || COMMAND_PROCESSING.load(Ordering::SeqCst)
                || PTT_HELD.load(Ordering::SeqCst);
            if busy {
                enqueue_tap(TapCmd::Cancel);
                return swallow_event();
            }
            return event;
        }

        // Command Mode: hold Command+Control+Option (or Fn+Control), speak an
        // instruction, then release to transform selected text — or generate at
        // the cursor when nothing is selected.
        if etype == K_CG_EVENT_FLAGS_CHANGED {
            let cmd_ctrl_opt = whimpr_core::hotkey_binding::flags::COMMAND
                | whimpr_core::hotkey_binding::flags::CONTROL
                | whimpr_core::hotkey_binding::flags::ALT;
            let fn_ctrl = whimpr_core::hotkey_binding::flags::FN
                | whimpr_core::hotkey_binding::flags::CONTROL;
            let command_chord = flags & cmd_ctrl_opt == cmd_ctrl_opt
                || (flags & fn_ctrl == fn_ctrl && (flags & FLAG_SECONDARY_FN) != 0);
            if command_chord {
                enqueue_tap(TapCmd::CommandDown);
                return swallow_event();
            }
            if COMMAND_HELD.load(Ordering::SeqCst) {
                enqueue_tap(TapCmd::CommandUp);
                return swallow_event();
            }
        }

        if binding.is_fn {
            if etype == K_CG_EVENT_FLAGS_CHANGED && keycode == KEYCODE_FN {
                let down = (flags & FLAG_SECONDARY_FN) != 0;
                if down {
                    if !PTT_HELD.swap(true, Ordering::SeqCst) {
                        enqueue_tap(TapCmd::PttDown { at_ms });
                    }
                } else if PTT_HELD.swap(false, Ordering::SeqCst) {
                    enqueue_tap(TapCmd::PttUp { at_ms });
                }
                return swallow_event();
            }
            return event;
        }

        // Modifier-only hold (e.g. right-option).
        if binding.modifiers == 0 && binding.keycode == keycode as u32 {
            if etype == K_CG_EVENT_FLAGS_CHANGED {
                let down = (flags & whimpr_core::hotkey_binding::flags::ALT) != 0
                    || (flags & whimpr_core::hotkey_binding::flags::CONTROL) != 0;
                if down {
                    if !PTT_HELD.swap(true, Ordering::SeqCst) {
                        enqueue_tap(TapCmd::PttDown { at_ms });
                    }
                } else if PTT_HELD.swap(false, Ordering::SeqCst) {
                    enqueue_tap(TapCmd::PttUp { at_ms });
                }
                return swallow_event();
            }
            return event;
        }

        // Key + modifier combo (e.g. option+w) — swallow so Option+letter doesn't type ∑, Ω, etc.
        // Keep this path tiny: only atomics + channel send. Heavy work runs on the worker.
        if keycode == binding.keycode as i64 {
            let mods_ok = whimpr_core::hotkey_binding::modifiers_match(binding.modifiers, flags);
            match etype {
                K_CG_EVENT_KEY_DOWN if mods_ok => {
                    if !PTT_HELD.swap(true, Ordering::SeqCst) {
                        enqueue_tap(TapCmd::PttDown { at_ms });
                    }
                    return swallow_event();
                }
                K_CG_EVENT_KEY_UP if mods_ok || PTT_HELD.load(Ordering::SeqCst) => {
                    if PTT_HELD.swap(false, Ordering::SeqCst) {
                        enqueue_tap(TapCmd::PttUp { at_ms });
                    }
                    return swallow_event();
                }
                _ => {}
            }
        }

        event
    }

    /// Load the speech model after startup or a completed first-run download.
    pub fn load_asr_model() {
        if ASR_LOADING.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| {
            let mut engines: Vec<Arc<dyn AsrEngine>> = Vec::new();
            let parakeet_path = crate::model_manager::parakeet_dir();
            if parakeet_path.exists() {
                match whimpr_asr::ParakeetEngine::load(&parakeet_path) {
                    Ok(engine) => engines.push(Arc::new(engine)),
                    Err(error) => {
                        eprintln!("[whimpr] Parakeet load failed, trying Whisper: {error}");
                    }
                }
            }
            let whisper_path = model_path();
            if whisper_path.exists() {
                match whimpr_asr::WhisperEngine::load(&whisper_path) {
                    Ok(engine) => engines.push(Arc::new(engine)),
                    Err(error) => {
                        eprintln!("[whimpr] Whisper ASR load failed: {error}");
                    }
                }
            }
            if engines.is_empty() {
                eprintln!("[whimpr] no ASR model found");
            } else if let Ok(fallback) = whimpr_asr::FallbackEngine::new(engines) {
                let engine: Arc<dyn AsrEngine> = Arc::new(fallback);
                if let Err(error) = engine.set_language(&current_settings().dictation_language) {
                    eprintln!("[whimpr] ASR language setup failed: {error}");
                }
                let id = engine.id();
                let slot = ASR.get_or_init(|| RwLock::new(None));
                if let Ok(mut current) = slot.write() {
                    *current = Some(engine);
                }
                eprintln!("[whimpr] ASR ready: {id:?}");
            }

            let vad_path = crate::model_manager::vad_path();
            if VAD.get().is_none() && vad_path.exists() {
                match whimpr_asr::SileroVadTrimmer::load(&vad_path) {
                    Ok(vad) => {
                        let _ = VAD.set(Arc::new(vad));
                        eprintln!("[whimpr] Silero VAD ready");
                    }
                    Err(error) => eprintln!("[whimpr] Silero VAD load failed: {error}"),
                }
            }
            ASR_LOADING.store(false, Ordering::SeqCst);
        });
    }

    pub fn install(app: AppHandle) {
        let _ = APP.set(app);
        let _ = MACHINE.set(Mutex::new(StateMachine::new()));
        let _ = CLOCK.set(Instant::now());

        // Load settings + dictionary, and build cloud providers from stored keys.
        let settings = whimpr_core::Settings::load(&settings_path());
        let dict = whimpr_core::DictionaryStore::load(&dict_path());
        let snippets = whimpr_core::SnippetStore::load(&snippets_path());
        let transforms = whimpr_core::TransformStore::load(&transforms_path());
        eprintln!(
            "[whimpr] cleanup mode: {:?}, level: {:?}",
            settings.cleanup_mode, settings.cleanup_level
        );
        let _ = SETTINGS.set(Mutex::new(settings));
        let _ = DICTIONARY.set(Mutex::new(dict));
        let _ = SNIPPETS.set(Mutex::new(snippets));
        let _ = TRANSFORMS.set(Mutex::new(transforms));
        let _ = STATS.set(Mutex::new(whimpr_core::StatsStore::load(&stats_path())));
        prune_history();
        rebuild_providers();
        load_asr_model();

        // Start the local cleanup worker in the background (model load takes a few
        // seconds; the first local cleanup waits for it, subsequent ones are fast).
        std::thread::spawn(|| {
            let worker = crate::local_llm::spawn_default();
            let _ = LOCAL.set(Mutex::new(worker));
        });

        // Accessibility is the ONE permission that makes the Fn CGEventTap global AND
        // lets us post the Cmd+V paste into other apps. Without it, a keyboard tap is
        // silently limited to frontmost-only — the exact bug. Prompt for it up front.
        if crate::paste::is_trusted() {
            eprintln!("[whimpr] Accessibility granted — Fn works in every app, paste enabled");
        } else {
            eprintln!(
                "[whimpr] ⚠ Accessibility NOT granted — Fn only works while WhimprFlow is \
                 frontmost and paste is disabled. Prompting; grant WhimprFlow under System \
                 Settings → Privacy & Security → Accessibility (no relaunch needed)."
            );
            crate::paste::prompt_accessibility();
        }
        // Input Monitoring is NOT the gate for a CGEventTap — kept only as diagnostics.
        eprintln!(
            "[whimpr] (info) Input Monitoring: {}",
            crate::paste::input_monitoring_granted()
        );

        // Periodic tick drives the double-tap timeout / session cap.
        std::thread::spawn(|| loop {
            std::thread::sleep(Duration::from_millis(100));
            handle_input(Input::Tick { now_ms: now_ms() });
        });

        start_tap_worker();

        // The event tap runs on a thread with its own CFRunLoop. CRITICAL: create it
        // ONLY after the process is trusted for Accessibility. macOS fixes a keyboard
        // tap's privilege at CGEventTapCreate time — a tap born untrusted is
        // permanently frontmost-only and is NOT upgraded when the grant later arrives.
        // Polling here also means the Fn key starts working the moment the user grants
        // Accessibility, without a relaunch.
        std::thread::spawn(|| {
            while !crate::paste::is_trusted() {
                std::thread::sleep(Duration::from_millis(500));
            }
            eprintln!("[whimpr] Accessibility present — creating global PTT tap");
            let port = unsafe {
                CGEventTapCreate(
                    K_CG_SESSION_EVENT_TAP,
                    K_CG_HEAD_INSERT,
                    K_CG_TAP_OPTION_DEFAULT,
                    EVENTS_OF_INTEREST,
                    tap_callback,
                    null_mut(),
                )
            };
            if port.is_null() {
                eprintln!(
                    "[whimpr] Fn tap null despite Accessibility — likely a stale TCC entry from \
                     an earlier build. Run: tccutil reset Accessibility com.whimpr.whimprflow, \
                     then re-grant and relaunch."
                );
                return;
            }
            TAP_PORT.store(port, Ordering::SeqCst);
            unsafe {
                let source = CFMachPortCreateRunLoopSource(null(), port, 0);
                CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopDefaultMode);
                CGEventTapEnable(port, true);
                CFRunLoopRun();
            }
        });
    }

    /// True once an ASR model is loaded into memory.
    pub fn asr_ready() -> bool {
        ASR.get()
            .and_then(|slot| slot.read().ok().map(|engine| engine.is_some()))
            .unwrap_or(false)
    }
}

#[cfg(target_os = "macos")]
pub use imp::{
    asr_ready, clear_history, current_settings, delete_history, dictionary_add, dictionary_entries,
    dictionary_learn, dictionary_remove, history, history_audio, install, language_stats,
    load_asr_model, rebuild_providers, reload_local_worker, sessions_for_analysis, snippet_add,
    snippet_remove, snippets_list, stats_summary, transform_remove, transform_upsert,
    transforms_list, update_settings,
};

// Windows uses the real (but unverified) platform layer in `crate::win`.
#[cfg(target_os = "windows")]
pub use crate::win::{
    asr_ready, clear_history, current_settings, delete_history, dictionary_add, dictionary_entries,
    dictionary_learn, dictionary_remove, history, history_audio, install, load_asr_model,
    rebuild_providers, reload_local_worker, sessions_for_analysis, stats_summary, update_settings,
};

// Other platforms (Linux, etc.): inert stubs so the crate still builds.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod other {
    pub fn install(_app: tauri::AppHandle) {}
    pub fn load_asr_model() {}
    pub fn asr_ready() -> bool {
        false
    }
    pub fn current_settings() -> whimpr_core::Settings {
        whimpr_core::Settings::default()
    }
    pub fn update_settings(_new: whimpr_core::Settings) {}
    pub fn rebuild_providers() {}
    pub fn reload_local_worker() {}
    pub fn stats_summary(tz_offset_minutes: i32) -> whimpr_core::StatsSummary {
        whimpr_core::StatsStore::default().summary(tz_offset_minutes, 0)
    }
    pub fn history(_limit: usize) -> Vec<whimpr_core::HistoryItem> {
        Vec::new()
    }
    pub fn dictionary_entries() -> Vec<super::DictEntryDto> {
        Vec::new()
    }
    pub fn delete_history(_ts_unix: u64) -> bool {
        false
    }
    pub fn clear_history() -> usize {
        0
    }
    pub fn prune_history() {}
    pub fn history_audio(_ts_unix: u64) -> Option<Vec<u8>> {
        None
    }
    pub fn sessions_for_analysis(_limit: usize) -> Vec<(String, Option<String>)> {
        Vec::new()
    }
    pub fn snippets_list() -> Vec<whimpr_core::Snippet> {
        Vec::new()
    }
    pub fn snippet_add(_trigger: String, _expansion: String) {}
    pub fn snippet_remove(_trigger: &str) -> bool {
        false
    }
    pub fn transforms_list() -> Vec<whimpr_core::TransformPreset> {
        Vec::new()
    }
    pub fn transform_upsert(_preset: whimpr_core::TransformPreset) {}
    pub fn transform_remove(_id: &str) -> bool {
        false
    }
    pub fn language_stats(_limit: usize) -> whimpr_core::LanguageStats {
        whimpr_core::LanguageStats::default()
    }
    pub fn dictionary_add(_correct: String, _mishears: Vec<String>) {}
    pub fn dictionary_remove(_correct: &str) {}
    pub fn dictionary_learn(_correct: String, _mishears: Vec<String>) {}
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub use other::{
    asr_ready, clear_history, current_settings, delete_history, dictionary_add, dictionary_entries,
    dictionary_learn, dictionary_remove, history, history_audio, install, language_stats,
    load_asr_model, rebuild_providers, reload_local_worker, sessions_for_analysis, snippet_add,
    snippet_remove, snippets_list, stats_summary, transform_remove, transform_upsert,
    transforms_list, update_settings,
};
