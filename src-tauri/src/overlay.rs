//! Flow Bar overlay — always-on-top, non-activating pill host.
//!
//! Uses a normal Tauri `WebviewWindow` (same approach as upstream WhimprFlow /
//! Handy). An earlier NSPanel path dropped state/waveform events, leaving the
//! pill stuck on the idle dot.
//!
//! A polled in-process snapshot remains as a backup so the UI still updates if
//! a Tauri event is missed.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Monitor, Position, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

pub const OVERLAY_LABEL: &str = "whimpr_bar";

const OVERLAY_W: f64 = 360.0;
const OVERLAY_H: f64 = 90.0;
/// Logical pt below the top of the monitor — sits in the notch divot on iMacs.
const TOP_INSET: f64 = 52.0;
const FOLLOW_INTERVAL: Duration = Duration::from_millis(250);
const WAVEFORM_EVAL_MIN: Duration = Duration::from_millis(33);

static FOLLOW_STARTED: AtomicBool = AtomicBool::new(false);
static VISIBLE: AtomicBool = AtomicBool::new(true);
static LAST_TARGET_MONITOR: OnceLock<Mutex<Option<(i32, i32)>>> = OnceLock::new();
static LAST_FRONTMOST_FRAME: OnceLock<Mutex<Option<(i32, i32, i32, i32)>>> = OnceLock::new();
static SNAP: OnceLock<Mutex<FlowBarSnap>> = OnceLock::new();
static SNAP_EPOCH: AtomicU64 = AtomicU64::new(1);
static LAST_WAVE_EVAL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Latest Flow Bar UI payload — polled by the overlay webview.
#[derive(Debug, Clone, Serialize)]
pub struct FlowBarSnap {
    pub state: String,
    pub message: Option<String>,
    pub bars: Vec<f32>,
    pub preview: String,
    pub epoch: u64,
}

impl Default for FlowBarSnap {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            message: None,
            bars: vec![0.0; 16],
            preview: String::new(),
            epoch: 0,
        }
    }
}

fn snap_slot() -> &'static Mutex<FlowBarSnap> {
    SNAP.get_or_init(|| Mutex::new(FlowBarSnap::default()))
}

fn bump_epoch() -> u64 {
    SNAP_EPOCH.fetch_add(1, Ordering::SeqCst) + 1
}

/// Current Flow Bar snapshot for the overlay poll loop.
pub fn snapshot() -> FlowBarSnap {
    snap_slot().lock().map(|s| s.clone()).unwrap_or_default()
}

pub fn build_overlay(app: &tauri::App) -> tauri::Result<WebviewWindow> {
    let (x, y) = overlay_position(app.handle(), OVERLAY_W, OVERLAY_H).unwrap_or((200.0, 200.0));
    let mut builder =
        WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App("overlay.html".into()))
            .title("WhimprBar")
            .inner_size(OVERLAY_W, OVERLAY_H)
            .position(x, y)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .focused(false)
            .resizable(false)
            .visible(true);

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .accept_first_mouse(true)
            .visible_on_all_workspaces(true)
            .focusable(false);
    }

    #[cfg(not(target_os = "macos"))]
    {
        builder = builder.accept_first_mouse(true).focusable(false);
    }

    let overlay = builder.build()?;
    present_on(app.handle(), &overlay);
    Ok(overlay)
}

/// Re-anchor and re-assert z-order before every pill state change.
pub fn present(app: &AppHandle) {
    if !VISIBLE.load(Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let app2 = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if let Some(w) = overlay_window(&app2) {
            present_on(&app2, &w);
        } else {
            eprintln!("[whimpr] overlay window missing (label={OVERLAY_LABEL})");
        }
    }) {
        eprintln!("[whimpr] overlay present failed to schedule on main thread: {e}");
    }
}

pub fn is_enabled() -> bool {
    VISIBLE.load(Ordering::SeqCst)
}

pub fn set_enabled(app: &AppHandle, enabled: bool) {
    VISIBLE.store(enabled, Ordering::SeqCst);
    let app = app.clone();
    let app_for_task = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        let Some(window) = overlay_window(&app_for_task) else {
            return;
        };
        if enabled {
            present_on(&app_for_task, &window);
        } else {
            let _ = window.hide();
        }
    }) {
        eprintln!("[whimpr] overlay visibility scheduling failed: {error}");
    }
}

/// Keep the pill anchored to the display containing the frontmost app.
pub fn start_monitor_follow(app: AppHandle) {
    if FOLLOW_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Ok(monitors) = app.available_monitors() {
        for (index, monitor) in monitors.iter().enumerate() {
            eprintln!(
                "[whimpr] display[{index}] name={:?} pos=({}, {}) size={}x{} scale={}",
                monitor.name(),
                monitor.position().x,
                monitor.position().y,
                monitor.size().width,
                monitor.size().height,
                monitor.scale_factor()
            );
        }
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(FOLLOW_INTERVAL);
        let app_for_task = app.clone();
        if app
            .run_on_main_thread(move || {
                if VISIBLE.load(Ordering::SeqCst) {
                    if let Some(window) = overlay_window(&app_for_task) {
                        reposition_on_active_monitor(&app_for_task, &window, false);
                    }
                }
            })
            .is_err()
        {
            break;
        }
    });
}

pub fn dispatch_state(app: &AppHandle, state: &str, message: Option<String>) {
    let epoch = bump_epoch();
    if let Ok(mut snap) = snap_slot().lock() {
        snap.state = state.to_string();
        snap.message = message.clone();
        snap.epoch = epoch;
        if !matches!(state, "recording" | "locked" | "listening") {
            snap.preview.clear();
            snap.bars = vec![0.0; 16];
        }
    }
    let payload = serde_json::json!({ "state": state, "message": message, "epoch": epoch });
    let _ = app.emit(OVERLAY_LABEL_EVENT_STATE, &payload);
    let _ = app.emit_to(OVERLAY_LABEL, OVERLAY_LABEL_EVENT_STATE, &payload);
    push_dom(app, "__WHIMPR_OVERLAY_STATE__", "whimpr:overlay-state", payload);
}

pub fn dispatch_waveform(app: &AppHandle, bars: &[f32]) {
    let epoch = bump_epoch();
    if let Ok(mut snap) = snap_slot().lock() {
        snap.bars = bars.to_vec();
        snap.epoch = epoch;
        if snap.state == "idle" || snap.state == "done" || snap.state == "cancelled" {
            snap.state = "recording".into();
            snap.message = None;
        }
    }

    let payload = serde_json::json!({ "bars": bars });
    // Always emit Tauri events (cheap). Rate-limit only the heavier DOM eval.
    let _ = app.emit(OVERLAY_LABEL_EVENT_WAVE, &payload);
    let _ = app.emit_to(OVERLAY_LABEL, OVERLAY_LABEL_EVENT_WAVE, &payload);

    let slot = LAST_WAVE_EVAL.get_or_init(|| Mutex::new(None));
    let mut last = slot.lock().unwrap();
    let now = Instant::now();
    if last.is_some_and(|t| now.duration_since(t) < WAVEFORM_EVAL_MIN) {
        return;
    }
    *last = Some(now);
    drop(last);

    push_dom(app, "__WHIMPR_OVERLAY_WAVEFORM__", "whimpr:overlay-waveform", payload);
}

pub fn dispatch_partial(app: &AppHandle, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let epoch = bump_epoch();
    if let Ok(mut snap) = snap_slot().lock() {
        snap.preview = trimmed.to_string();
        snap.epoch = epoch;
        if snap.state == "idle" {
            snap.state = "recording".into();
        }
    }
    let payload = serde_json::json!({ "text": trimmed });
    let _ = app.emit(OVERLAY_LABEL_EVENT_PARTIAL, &payload);
    let _ = app.emit_to(OVERLAY_LABEL, OVERLAY_LABEL_EVENT_PARTIAL, &payload);
    push_dom(app, "__WHIMPR_OVERLAY_PARTIAL__", "whimpr:overlay-partial", payload);
}

const OVERLAY_LABEL_EVENT_STATE: &str = "whimpr://flowbar/state";
const OVERLAY_LABEL_EVENT_WAVE: &str = "whimpr://audio/waveform";
const OVERLAY_LABEL_EVENT_PARTIAL: &str = "whimpr://flowbar/partial";

fn push_dom(app: &AppHandle, global: &str, event: &str, payload: serde_json::Value) {
    let kind = match global {
        "__WHIMPR_OVERLAY_STATE__" => "state",
        "__WHIMPR_OVERLAY_WAVEFORM__" => "waveform",
        "__WHIMPR_OVERLAY_PARTIAL__" => "partial",
        _ => "snap",
    };
    let (Ok(global_js), Ok(event_js), Ok(kind_js)) = (
        serde_json::to_string(global),
        serde_json::to_string(event),
        serde_json::to_string(kind),
    ) else {
        return;
    };
    // Prefer the imperative bridge (`__whimprApply`) so updates land even if
    // React listeners aren't mounted yet. Keep the CustomEvent + global as
    // fallbacks for older overlay bundles.
    let script = format!(
        "(function(){{\
           window[{global_js}] = {payload};\
           try {{\
             if (typeof window.__whimprApply === 'function') {{\
               window.__whimprApply({kind_js}, {payload});\
             }}\
           }} catch (e) {{ console.error('[whimpr] __whimprApply', e); }}\
           try {{\
             window.dispatchEvent(new CustomEvent({event_js}, {{ detail: {payload} }}));\
           }} catch (e) {{}}\
         }})();"
    );
    let app = app.clone();
    let app_for_task = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        let Some(window) = overlay_window(&app_for_task) else {
            eprintln!("[whimpr] overlay eval skipped — window missing");
            return;
        };
        if let Err(err) = window.eval(&script) {
            eprintln!("[whimpr] overlay eval failed: {err}");
        }
    }) {
        eprintln!("[whimpr] overlay eval schedule failed: {e}");
    }
}

fn overlay_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(OVERLAY_LABEL)
}

fn present_on(app: &AppHandle, w: &WebviewWindow) {
    // Move only when the active display changes — calling set_size every few
    // hundred ms was freezing the overlay webview (idle CSS kept pulsing).
    reposition_on_active_monitor(app, w, false);
    let _ = w.set_visible_on_all_workspaces(true);
    let _ = w.set_always_on_top(true);
    let _ = w.show();
    raise_native(w);
}

#[cfg(target_os = "macos")]
fn raise_native(w: &WebviewWindow) {
    use objc2_app_kit::{NSStatusWindowLevel, NSWindow};
    let Ok(ptr) = w.ns_window() else {
        return;
    };
    if ptr.is_null() {
        return;
    }
    // SAFETY: Tauri owns this NSWindow for the lifetime of the WebviewWindow.
    unsafe {
        let ns_window = &*(ptr as *const NSWindow);
        ns_window.setLevel(NSStatusWindowLevel);
        ns_window.orderFrontRegardless();
    }
}

#[cfg(not(target_os = "macos"))]
fn raise_native(_w: &WebviewWindow) {}

fn reposition_on_active_monitor(app: &AppHandle, window: &WebviewWindow, force: bool) {
    let Some(monitor) = active_monitor(app) else {
        return;
    };
    let key = (monitor.position().x, monitor.position().y);
    let target = LAST_TARGET_MONITOR.get_or_init(|| Mutex::new(None));
    let mut previous = target.lock().unwrap();
    let changed = *previous != Some(key);
    if !force && !changed {
        return;
    }
    if changed {
        eprintln!(
            "[whimpr] pill monitor -> {:?} @({}, {})",
            monitor.name(),
            key.0,
            key.1
        );
        *previous = Some(key);
    }
    drop(previous);

    let (x, y) = top_center_on_monitor(&monitor, OVERLAY_W);
    let _ = window.set_position(Position::Logical(LogicalPosition { x, y }));
    let _ = window.set_size(tauri::Size::Logical(LogicalSize {
        width: OVERLAY_W,
        height: OVERLAY_H,
    }));
}

fn monitor_containing_logical_point(app: &AppHandle, x: f64, y: f64) -> Option<Monitor> {
    app.available_monitors().ok()?.into_iter().find(|monitor| {
        let scale = monitor.scale_factor();
        let origin_x = monitor.position().x as f64 / scale;
        let origin_y = monitor.position().y as f64 / scale;
        let width = monitor.size().width as f64 / scale;
        let height = monitor.size().height as f64 / scale;
        x >= origin_x && x < origin_x + width && y >= origin_y && y < origin_y + height
    })
}

/// Prefer the frontmost app's window, then the cursor, then the primary display.
fn active_monitor(app: &AppHandle) -> Option<Monitor> {
    if let Some(frame) = crate::caret::frontmost_window_frame() {
        let frame_key = (
            frame.x.round() as i32,
            frame.y.round() as i32,
            frame.width.round() as i32,
            frame.height.round() as i32,
        );
        let previous = LAST_FRONTMOST_FRAME.get_or_init(|| Mutex::new(None));
        let mut previous = previous.lock().unwrap();
        if *previous != Some(frame_key) {
            eprintln!(
                "[whimpr] frontmost frame -> ({}, {}) {}x{}",
                frame_key.0, frame_key.1, frame_key.2, frame_key.3
            );
            *previous = Some(frame_key);
        }
        drop(previous);
        let (x, y) = crate::caret::rect_center(frame);
        if let Some(monitor) = monitor_containing_logical_point(app, x, y) {
            return Some(monitor);
        }
    }

    app.cursor_position()
        .ok()
        .and_then(|position| {
            app.monitor_from_point(position.x, position.y)
                .ok()
                .flatten()
        })
        .or_else(|| app.primary_monitor().ok().flatten())
        .or_else(|| app.available_monitors().ok()?.into_iter().next())
}

/// Top-center of `monitor`, inset below the menu bar / notch divot.
fn top_center_on_monitor(monitor: &Monitor, width: f64) -> (f64, f64) {
    let scale = monitor.scale_factor();
    let monitor_x = monitor.position().x as f64 / scale;
    let monitor_y = monitor.position().y as f64 / scale;
    let monitor_width = monitor.size().width as f64 / scale;
    let x = monitor_x + (monitor_width - width) / 2.0;
    let y = monitor_y + TOP_INSET;
    (x, y)
}

fn overlay_position(app: &AppHandle, width: f64, _height: f64) -> Option<(f64, f64)> {
    let monitor = active_monitor(app)?;
    Some(top_center_on_monitor(&monitor, width))
}
