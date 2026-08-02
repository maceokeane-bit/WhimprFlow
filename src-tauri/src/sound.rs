//! Short feedback sounds for dictation events.
//!
//! Three cues so you can follow the pipeline without watching the Flow Bar:
//! 1. Start recording — Ping
//! 2. Stop recording (key release) — Tink
//! 3. Text pasted / done — Pop

/// Play the record-start ping (caller checks settings).
pub fn play_record_ping() {
    play_named("Ping");
}

/// Play when push-to-talk is released and capture stops.
pub fn play_stop_ping() {
    play_named("Tink");
}

/// Play when cleanup finishes and text has been inserted.
pub fn play_done_ping() {
    play_named("Pop");
}

fn play_named(name: &'static str) {
    std::thread::spawn(move || {
        #[cfg(target_os = "macos")]
        mac::play(name);
        #[cfg(target_os = "windows")]
        {
            let _ = name;
            win::play_beep();
        }
    });
}

#[cfg(target_os = "macos")]
mod mac {
    use std::process::{Command, Stdio};

    pub fn play(name: &str) {
        let path = format!("/System/Library/Sounds/{name}.aiff");
        // Slightly quieter stop/done so the start ping stays the loudest cue.
        let volume = match name {
            "Ping" => "1",
            "Tink" => "0.85",
            _ => "0.9",
        };
        let _ = Command::new("afplay")
            .arg("-v")
            .arg(volume)
            .arg(&path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(target_os = "windows")]
mod win {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBeep, MB_ICONASTERISK};

    pub fn play_beep() {
        unsafe {
            let _ = MessageBeep(MB_ICONASTERISK);
        }
    }
}
