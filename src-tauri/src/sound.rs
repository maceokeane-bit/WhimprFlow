//! Short feedback sounds for dictation events.

/// Play the record-start ping (respects nothing here — caller checks settings).
pub fn play_record_ping() {
    std::thread::spawn(|| {
        #[cfg(target_os = "macos")]
        mac::play_pop();
        #[cfg(target_os = "windows")]
        win::play_beep();
    });
}

#[cfg(target_os = "macos")]
mod mac {
    use std::process::{Command, Stdio};

    /// macOS built-in "Pop" — short, unobtrusive, no bundled asset needed.
    const POP: &str = "/System/Library/Sounds/Pop.aiff";

    pub fn play_pop() {
        let _ = Command::new("afplay")
            .arg("-v")
            .arg("0.35")
            .arg(POP)
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
