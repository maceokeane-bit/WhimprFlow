//! Pause/resume system media while dictating (Spotify, Music, browser video, etc.).
//!
//! MediaRemote's "is playing" query is block-based and crashes if called with the
//! wrong signature, so we pause Spotify/Music via AppleScript and fall back to
//! MediaRemote send-command for browser/system now-playing (main thread only).

use std::sync::atomic::{AtomicU8, Ordering};

/// Bit flags for what we paused so resume only touches those targets.
const NONE: u8 = 0;
const SPOTIFY: u8 = 1;
const MUSIC: u8 = 2;
const MEDIA_REMOTE: u8 = 4;

static PAUSED_TARGETS: AtomicU8 = AtomicU8::new(NONE);

/// Called when microphone capture begins (must run on the main thread).
pub fn on_dictation_start() {
    #[cfg(target_os = "macos")]
    {
        let mut targets = NONE;
        if mac::script_pause_spotify() {
            targets |= SPOTIFY;
        }
        if mac::script_pause_music() {
            targets |= MUSIC;
        }
        // Browser / Podcasts / etc. — pause the system now-playing client.
        if targets == NONE && mac::send_pause_command() {
            targets |= MEDIA_REMOTE;
        }
        if targets != NONE {
            PAUSED_TARGETS.store(targets, Ordering::SeqCst);
            eprintln!("[whimpr] paused media (targets={targets:#x})");
        }
    }
}

/// Called when capture ends (finalize, discard, or cancel). Main thread only.
pub fn on_dictation_stop() {
    let targets = PAUSED_TARGETS.swap(NONE, Ordering::SeqCst);
    if targets == NONE {
        return;
    }
    #[cfg(target_os = "macos")]
    {
        if targets & SPOTIFY != 0 {
            mac::script_play_spotify();
        }
        if targets & MUSIC != 0 {
            mac::script_play_music();
        }
        if targets & MEDIA_REMOTE != 0 {
            mac::send_play_command();
        }
        eprintln!("[whimpr] resumed media (targets={targets:#x})");
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::{c_char, c_void};
    use std::process::Command;
    use std::sync::OnceLock;

    const CMD_PLAY: u32 = 0;
    const CMD_PAUSE: u32 = 1;

    type SendCommandFn = unsafe extern "C" fn(u32, *const c_void) -> bool;

    fn send_command(cmd: u32) -> bool {
        static SEND: OnceLock<Option<SendCommandFn>> = OnceLock::new();
        let Some(send) = *SEND.get_or_init(load_send_command) else {
            return false;
        };
        unsafe { send(cmd, std::ptr::null()) }
    }

    fn load_send_command() -> Option<SendCommandFn> {
        unsafe {
            let framework = b"/System/Library/PrivateFrameworks/MediaRemote.framework/MediaRemote\0";
            let handle = dlopen(framework.as_ptr() as *const c_char, RTLD_LAZY);
            if handle.is_null() {
                return None;
            }
            let sym = dlsym(handle, b"MRMediaRemoteSendCommand\0".as_ptr() as *const c_char);
            if sym.is_null() {
                return None;
            }
            Some(std::mem::transmute(sym))
        }
    }

    pub fn send_pause_command() -> bool {
        send_command(CMD_PAUSE)
    }

    pub fn send_play_command() -> bool {
        send_command(CMD_PLAY)
    }

    pub fn script_pause_spotify() -> bool {
        run_osascript_bool(
            r#"
            try
                tell application "Spotify"
                    if player state is playing then
                        pause
                        return true
                    end if
                end tell
            end try
            return false
            "#,
        )
    }

    pub fn script_pause_music() -> bool {
        run_osascript_bool(
            r#"
            try
                tell application "Music"
                    if player state is playing then
                        pause
                        return true
                    end if
                end tell
            end try
            return false
            "#,
        )
    }

    pub fn script_play_spotify() -> bool {
        run_osascript_bool(
            r#"
            try
                tell application "Spotify"
                    if player state is paused then
                        play
                        return true
                    end if
                end tell
            end try
            return false
            "#,
        )
    }

    pub fn script_play_music() -> bool {
        run_osascript_bool(
            r#"
            try
                tell application "Music"
                    if player state is paused then
                        play
                        return true
                    end if
                end tell
            end try
            return false
            "#,
        )
    }

    fn run_osascript_bool(script: &str) -> bool {
        Command::new("osascript")
            .args(["-e", script])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim() == "true")
            .unwrap_or(false)
    }

    const RTLD_LAZY: i32 = 0x1;

    #[link(name = "dl")]
    extern "C" {
        fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
}
