//! WhimprFlow Tauri shell.
//!
//! Runs as a macOS accessory (menu-bar) app: a tray item, a transparent
//! always-on-top Flow Bar overlay, and a hidden Hub window. This is the M0
//! skeleton — the sidecar supervisor, real state-machine bridge, and native
//! panel promotion arrive in later milestones. The overlay already listens for
//! `whimpr://flowbar/state`, so the tray demo items prove the event pipeline.

mod appctx;
mod audio_archive;
mod autolearn;
mod caret;
mod cleanup_model_manager;
mod context;
mod hotkey;
mod insights;
mod local_llm;
mod media;
mod model_manager;
mod overlay;
mod paste;
mod services;
mod sound;
mod transforms;
#[cfg(target_os = "windows")]
mod win;

use serde::Serialize;
use std::sync::OnceLock;
use tauri::{
    menu::{ContextMenu, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

use overlay::OVERLAY_LABEL;

const HUB_LABEL: &str = "main";
static FLOW_MENU: OnceLock<Menu<tauri::Wry>> = OnceLock::new();

fn build_overlay(app: &tauri::App) -> tauri::Result<tauri::WebviewWindow> {
    overlay::build_overlay(app)
}

fn build_hub(app: &tauri::App) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, HUB_LABEL, WebviewUrl::App("index.html".into()))
        .title("WhimprFlow")
        .inner_size(920.0, 640.0)
        .min_inner_size(720.0, 480.0)
        .visible(true)
        .build()
}

fn show_hub(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(HUB_LABEL) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn navigate_hub(app: &tauri::AppHandle, page: &str) {
    show_hub(app);
    let _ = app.emit_to(HUB_LABEL, "whimpr://hub/navigate", page);
}

fn build_flow_menu(app: &tauri::App) -> tauri::Result<Menu<tauri::Wry>> {
    let open = MenuItem::with_id(app, "open", "Open WhimprFlow", true, None::<&str>)?;
    let paste_last = MenuItem::with_id(
        app,
        "paste_last",
        "Paste Last Transcript",
        true,
        None::<&str>,
    )?;
    let settings = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let microphone = MenuItem::with_id(app, "microphone", "Microphone", true, None::<&str>)?;
    let languages = MenuItem::with_id(app, "languages", "Languages", true, None::<&str>)?;
    let transforms = MenuItem::with_id(app, "transforms", "Transforms", true, None::<&str>)?;
    let history = MenuItem::with_id(app, "history", "Transcript History", true, None::<&str>)?;
    let toggle_bar = MenuItem::with_id(
        app,
        "toggle_bar",
        "Show / Hide Flow Bar",
        true,
        None::<&str>,
    )?;
    let snooze = MenuItem::with_id(app, "snooze_bar", "Hide for 1 Hour", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let separator2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit WhimprFlow", true, None::<&str>)?;
    Menu::with_items(
        app,
        &[
            &open,
            &paste_last,
            &separator,
            &settings,
            &microphone,
            &languages,
            &transforms,
            &history,
            &toggle_bar,
            &snooze,
            &separator2,
            &quit,
        ],
    )
}

fn handle_flow_menu_event(app: &tauri::AppHandle, event: MenuEvent) {
    match event.id.as_ref() {
        "open" => show_hub(app),
        "settings" | "languages" => navigate_hub(app, "settings"),
        "transforms" => navigate_hub(app, "transforms"),
        "history" => navigate_hub(app, "home"),
        "microphone" => {
            request_microphone();
            navigate_hub(app, "settings");
        }
        "paste_last" => {
            if let Some(item) = hotkey::history(1).first() {
                let _ = paste::paste_text(&item.text);
            }
        }
        "toggle_bar" => toggle_flow_bar(app),
        "snooze_bar" => snooze_flow_bar(app),
        "quit" => app.exit(0),
        _ => {}
    }
}

#[tauri::command]
fn show_flow_menu(app: tauri::AppHandle) -> Result<(), String> {
    let menu = FLOW_MENU
        .get()
        .ok_or_else(|| "Flow Menu is not ready".to_string())?;
    let window = app
        .get_webview_window(OVERLAY_LABEL)
        .ok_or_else(|| "Flow Bar window is unavailable".to_string())?;
    menu.popup(window.as_ref().window())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_settings() -> whimpr_core::Settings {
    hotkey::current_settings()
}

#[tauri::command]
fn set_settings(app: tauri::AppHandle, settings: whimpr_core::Settings) {
    let launch = settings.launch_at_login;
    hotkey::update_settings(settings.clone());
    sync_launch_at_login(&app, launch);
    apply_flow_bar_visibility(&app, &settings);
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn apply_flow_bar_visibility(app: &tauri::AppHandle, settings: &whimpr_core::Settings) {
    let now = unix_now();
    let snoozed_until = settings.flow_bar_snoozed_until.filter(|until| *until > now);
    let visible = settings.show_flow_bar && snoozed_until.is_none();
    let was_visible = overlay::is_enabled();
    overlay::set_enabled(app, visible);
    if visible && !was_visible {
        overlay::dispatch_state(app, "idle", None);
    }

    if let Some(until) = snoozed_until {
        let app = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(
                until.saturating_sub(unix_now()) + 1,
            ));
            let mut current = hotkey::current_settings();
            if current.flow_bar_snoozed_until == Some(until) && unix_now() >= until {
                current.flow_bar_snoozed_until = None;
                hotkey::update_settings(current.clone());
                overlay::set_enabled(&app, current.show_flow_bar);
                if current.show_flow_bar {
                    overlay::dispatch_state(&app, "idle", None);
                }
            }
        });
    }
}

fn toggle_flow_bar(app: &tauri::AppHandle) {
    let mut settings = hotkey::current_settings();
    settings.show_flow_bar = !settings.show_flow_bar;
    if settings.show_flow_bar {
        settings.flow_bar_snoozed_until = None;
    }
    hotkey::update_settings(settings.clone());
    apply_flow_bar_visibility(app, &settings);
}

fn snooze_flow_bar(app: &tauri::AppHandle) {
    let mut settings = hotkey::current_settings();
    settings.flow_bar_snoozed_until = Some(unix_now() + 60 * 60);
    hotkey::update_settings(settings.clone());
    apply_flow_bar_visibility(app, &settings);
}

/// Aggregated dictation stats for the Hub dashboard. `tz_offset_minutes` is the
/// browser's `Date.getTimezoneOffset()` so "today"/streak match the user's clock.
#[tauri::command]
fn get_stats(tz_offset_minutes: i32) -> whimpr_core::StatsSummary {
    hotkey::stats_summary(tz_offset_minutes)
}

/// Recent dictations for the Hub Home history list (newest first).
#[tauri::command]
fn get_history() -> Vec<whimpr_core::HistoryItem> {
    hotkey::history(200)
}

/// Dictionary entries for the Hub Dictionary screen.
#[tauri::command]
fn get_dictionary() -> Vec<hotkey::DictEntryDto> {
    hotkey::dictionary_entries()
}

/// Add a manual dictionary entry (word + optional known mishears).
#[tauri::command]
fn add_dictionary_entry(correct: String, mishears: Vec<String>) {
    hotkey::dictionary_add(correct, mishears);
}

/// Remove a dictionary entry by its spelling.
#[tauri::command]
fn remove_dictionary_entry(correct: String) {
    hotkey::dictionary_remove(&correct);
}

/// Permission + capability status shown in the Hub.
#[derive(Clone, Serialize)]
struct StatusReport {
    accessibility: bool,
    microphone: bool,
    input_monitoring: bool,
    has_openai_key: bool,
    has_anthropic_key: bool,
}

#[tauri::command]
fn get_status() -> StatusReport {
    StatusReport {
        accessibility: paste::is_trusted(),
        microphone: paste::microphone_granted(),
        input_monitoring: paste::input_monitoring_granted(),
        has_openai_key: has_key("openai_api_key"),
        has_anthropic_key: has_key("anthropic_api_key"),
    }
}

fn has_key(account: &str) -> bool {
    keyring::Entry::new("com.whimpr.whimprflow", account)
        .ok()
        .and_then(|e| e.get_password().ok())
        .map(|k| !k.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn open_url(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}

/// Request microphone access: trigger the native prompt (bundle has a usage string)
/// by briefly opening the input device, and open the Microphone settings pane.
#[tauri::command]
fn request_microphone() {
    #[cfg(target_os = "macos")]
    {
        std::thread::spawn(|| {
            if let Ok(h) = whimpr_audio::start(|_: &[f32]| {}) {
                std::thread::sleep(std::time::Duration::from_millis(400));
                let _ = h.stop();
            }
        });
        open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone");
    }
}

#[derive(Clone, Serialize)]
struct MicrophoneTestResult {
    peak: f32,
    heard_voice: bool,
}

#[tauri::command]
async fn test_microphone() -> Result<MicrophoneTestResult, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let handle = whimpr_audio::start(|_: &[f32]| {}).map_err(|error| error.to_string())?;
        std::thread::sleep(std::time::Duration::from_millis(1_600));
        let audio = handle
            .stop()
            .ok_or_else(|| "Microphone test did not capture audio".to_string())?;
        let peak = audio
            .samples
            .iter()
            .fold(0.0_f32, |max, sample| max.max(sample.abs()));
        Ok(MicrophoneTestResult {
            peak,
            heard_voice: peak >= 0.01,
        })
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn relaunch_app(app: tauri::AppHandle) {
    app.restart();
}

/// Request Accessibility — the permission that makes the Fn key work in every app and
/// lets us type into other apps. Fire the native prompt, then open the pane.
#[tauri::command]
fn request_accessibility() {
    #[cfg(target_os = "macos")]
    {
        let _ = paste::prompt_accessibility();
        open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility");
    }
}

/// Request Input Monitoring (needed for the Fn key to be seen in every app, not
/// just while WhimprFlow is frontmost): register + prompt, then open the pane.
#[tauri::command]
fn request_input_monitoring() {
    #[cfg(target_os = "macos")]
    {
        let _ = paste::request_input_monitoring();
        open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent");
    }
}

/// Save (or clear, when empty) an API key in the OS keychain, then rebuild providers
/// so it takes effect immediately.
#[tauri::command]
fn set_api_key(provider: String, key: String) -> Result<(), String> {
    let account = match provider.as_str() {
        "openai" => "openai_api_key",
        "anthropic" => "anthropic_api_key",
        _ => return Err(format!("unknown provider {provider}")),
    };
    let entry = keyring::Entry::new("com.whimpr.whimprflow", account).map_err(|e| e.to_string())?;
    let key = key.trim();
    // Delete any existing item first so the new one is created by (and readable to)
    // this app — a key added via the `security` CLI isn't readable by the app.
    let _ = entry.delete_credential();
    if !key.is_empty() {
        entry.set_password(key).map_err(|e| e.to_string())?;
    }
    hotkey::rebuild_providers();
    Ok(())
}

#[tauri::command]
fn delete_history(ts_unix: u64) -> bool {
    hotkey::delete_history(ts_unix)
}

#[tauri::command]
fn clear_history() -> usize {
    hotkey::clear_history()
}

#[tauri::command]
fn read_history_audio(ts_unix: u64) -> Option<Vec<u8>> {
    hotkey::history_audio(ts_unix)
}

#[tauri::command]
fn analyze_insights(force_refresh: bool) -> insights::InsightReport {
    let settings = hotkey::current_settings();
    let sessions = hotkey::sessions_for_analysis(50);
    insights::analyze(
        &sessions,
        &settings.ollama_base_url,
        &settings.ollama_model,
        force_refresh,
    )
}

#[tauri::command]
fn get_language_stats() -> whimpr_core::LanguageStats {
    hotkey::language_stats(100)
}

#[tauri::command]
fn get_snippets() -> Vec<whimpr_core::Snippet> {
    hotkey::snippets_list()
}

#[tauri::command]
fn add_snippet(trigger: String, expansion: String) {
    hotkey::snippet_add(trigger, expansion);
}

#[tauri::command]
fn remove_snippet(trigger: String) -> bool {
    hotkey::snippet_remove(&trigger)
}

#[tauri::command]
fn get_transforms() -> Vec<whimpr_core::TransformPreset> {
    hotkey::transforms_list()
}

#[tauri::command]
fn save_transform(preset: whimpr_core::TransformPreset) {
    hotkey::transform_upsert(preset);
}

#[tauri::command]
fn remove_transform(id: String) -> bool {
    hotkey::transform_remove(&id)
}

#[tauri::command]
fn run_transform(preset_id: String, instruction: Option<String>) -> Result<String, String> {
    let settings = hotkey::current_settings();
    let selected = crate::paste::copy_selection()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            "No text selected — highlight text in another app, then run the transform.".to_string()
        })?;

    let instruction = if let Some(custom) = instruction.filter(|s| !s.trim().is_empty()) {
        custom
    } else {
        hotkey::transforms_list()
            .into_iter()
            .find(|p| p.id == preset_id)
            .map(|p| p.instruction)
            .ok_or_else(|| format!("Unknown transform preset: {preset_id}"))?
    };

    let out = transforms::apply_via_ollama(
        &selected,
        &instruction,
        &settings.ollama_base_url,
        &settings.ollama_model,
    )?;
    crate::paste::paste_text(&out).map_err(|e| e.to_string())?;
    Ok(out)
}

#[tauri::command]
fn get_hotkey_presets() -> Vec<(String, String)> {
    whimpr_core::HOTKEY_PRESETS
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[tauri::command]
fn get_services() -> services::ServicesStatus {
    services::status(hotkey::asr_ready())
}

#[tauri::command]
fn get_model_download_status(app: tauri::AppHandle) -> model_manager::ModelDownloadStatus {
    model_manager::status(app)
}

#[tauri::command]
fn start_model_download(app: tauri::AppHandle) -> Result<(), String> {
    model_manager::start_download(app)
}

#[tauri::command]
fn cancel_model_download() {
    model_manager::cancel_download();
}

#[tauri::command]
fn get_cleanup_model_status(app: tauri::AppHandle) -> cleanup_model_manager::CleanupModelStatus {
    cleanup_model_manager::status(app)
}

#[tauri::command]
fn start_cleanup_model_download(app: tauri::AppHandle) -> Result<(), String> {
    cleanup_model_manager::start_download(app)
}

#[tauri::command]
fn cancel_cleanup_model_download() {
    cleanup_model_manager::cancel_download();
}

#[tauri::command]
fn start_ollama() -> Result<(), String> {
    services::start_ollama()
}

#[tauri::command]
fn pull_ollama_model(model: String) -> Result<(), String> {
    services::pull_ollama_model(&model)
}

fn sync_launch_at_login(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    let result = if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    if let Err(e) = result {
        eprintln!("[whimpr] launch-at-login sync failed: {e}");
    }
}

#[tauri::command]
fn set_launch_at_login(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    if enabled {
        app.autolaunch().enable().map_err(|e| e.to_string())?;
    } else {
        app.autolaunch().disable().map_err(|e| e.to_string())?;
    }
    let mut settings = hotkey::current_settings();
    settings.launch_at_login = enabled;
    hotkey::update_settings(settings);
    Ok(())
}

#[tauri::command]
fn is_launch_at_login_enabled(app: tauri::AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

pub fn run() {
    // Only one process may own the global hotkey and overlay. Multiple instances
    // can otherwise stack identical pills while different processes receive Fn.
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(hub) = app.get_webview_window(HUB_LABEL) {
                let _ = hub.show();
                let _ = hub.set_focus();
            }
            overlay::present(app);
        }))
        .plugin(tauri_plugin_autostart::Builder::new().build());
    #[cfg(target_os = "macos")]
    {
        builder = builder.plugin(tauri_nspanel::init());
    }
    builder
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            get_stats,
            get_history,
            get_dictionary,
            add_dictionary_entry,
            remove_dictionary_entry,
            get_status,
            get_services,
            get_model_download_status,
            start_model_download,
            cancel_model_download,
            get_cleanup_model_status,
            start_cleanup_model_download,
            cancel_cleanup_model_download,
            start_ollama,
            pull_ollama_model,
            set_launch_at_login,
            is_launch_at_login_enabled,
            delete_history,
            clear_history,
            read_history_audio,
            analyze_insights,
            get_language_stats,
            get_snippets,
            add_snippet,
            remove_snippet,
            get_transforms,
            save_transform,
            remove_transform,
            run_transform,
            get_hotkey_presets,
            request_microphone,
            test_microphone,
            request_accessibility,
            request_input_monitoring,
            relaunch_app,
            show_flow_menu,
            set_api_key
        ])
        .on_menu_event(handle_flow_menu_event)
        .setup(|app| {
            // Menu-bar utility: Hub windows can still focus normally without a
            // permanent Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            build_overlay(app)?;
            overlay::start_monitor_follow(app.handle().clone());
            let hub = build_hub(app)?;
            let _ = hub.show();
            let _ = hub.set_focus();

            // Wire the Fn key to the pill via the real state machine.
            hotkey::install(app.handle().clone());

            // Honor saved launch-at-login preference.
            let settings = hotkey::current_settings();
            if settings.launch_at_login {
                sync_launch_at_login(app.handle(), true);
            }
            apply_flow_bar_visibility(app.handle(), &settings);

            let menu = build_flow_menu(app)?;
            let _ = FLOW_MENU.set(menu.clone());

            let mut tray = TrayIconBuilder::new()
                .menu(&menu)
                .show_menu_on_left_click(false);
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running WhimprFlow");
}
