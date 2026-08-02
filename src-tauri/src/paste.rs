//! Text insertion: deliver transcribed/cleaned text to the frontmost app.
//!
//! First rung of the insertion ladder — clipboard paste: save the current
//! clipboard, write our text, synthesize Cmd+V, then restore the clipboard. This
//! is the universal path that works in almost every app. (AX direct-insert and the
//! terminal/secure-input handling from the plan layer on later, in the sidecar.)
//!
//! Posting the Cmd+V keystroke requires **Accessibility** permission; [`is_trusted`]
//! reports whether it's granted so the shell can prompt.

#[cfg(target_os = "macos")]
mod imp {
    use std::os::raw::c_void;
    use std::ptr::null;
    use std::time::Duration;

    type CGEventRef = *mut c_void;
    type CGEventSourceRef = *const c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreateKeyboardEvent(
            source: CGEventSourceRef,
            keycode: u16,
            keydown: bool,
        ) -> CGEventRef;
        fn CGEventSetFlags(event: CGEventRef, flags: u64);
        fn CGEventPost(tap: u32, event: CGEventRef);
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    const KCG_HID_EVENT_TAP: u32 = 0;
    const KCG_FLAG_MASK_COMMAND: u64 = 0x0010_0000;
    const KEYCODE_V: u16 = 9;
    const KEYCODE_C: u16 = 8;

    pub fn input_monitoring_granted() -> bool {
        unsafe { CGPreflightListenEventAccess() }
    }

    pub fn request_input_monitoring() -> bool {
        unsafe { CGRequestListenEventAccess() }
    }

    pub fn is_trusted() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    pub fn prompt_accessibility() -> bool {
        macos_accessibility_client::accessibility::application_is_trusted_with_prompt()
    }

    pub fn microphone_granted() -> bool {
        use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
        unsafe {
            let Some(audio) = AVMediaTypeAudio else {
                return false;
            };
            let status = AVCaptureDevice::authorizationStatusForMediaType(audio);
            status == AVAuthorizationStatus::Authorized
        }
    }

    fn post_cmd(keycode: u16) {
        unsafe {
            let down = CGEventCreateKeyboardEvent(null(), keycode, true);
            CGEventSetFlags(down, KCG_FLAG_MASK_COMMAND);
            CGEventPost(KCG_HID_EVENT_TAP, down);
            CFRelease(down as *const c_void);

            let up = CGEventCreateKeyboardEvent(null(), keycode, false);
            CGEventSetFlags(up, KCG_FLAG_MASK_COMMAND);
            CGEventPost(KCG_HID_EVENT_TAP, up);
            CFRelease(up as *const c_void);
        }
    }

    fn post_cmd_v() {
        post_cmd(KEYCODE_V);
    }

    fn post_cmd_c() {
        post_cmd(KEYCODE_C);
    }

    /// Copy the current selection via Cmd+C and return clipboard text.
    pub fn copy_selection() -> anyhow::Result<Option<String>> {
        use arboard::Clipboard;
        if !is_trusted() {
            return Err(anyhow::anyhow!(
                "no Accessibility permission — cannot read selection"
            ));
        }
        let mut cb = Clipboard::new()?;
        let saved = cb.get_text().ok();
        post_cmd_c();
        std::thread::sleep(Duration::from_millis(120));
        let selected = cb.get_text().ok().filter(|t| !t.trim().is_empty());
        if let Some(prev) = saved {
            let _ = cb.set_text(prev);
        }
        Ok(selected)
    }

    pub fn paste_text(text: &str) -> anyhow::Result<()> {
        use arboard::Clipboard;
        if !is_trusted() {
            return Err(anyhow::anyhow!(
                "no Accessibility permission — cannot paste (grant it in System Settings → \
                 Privacy & Security → Accessibility, then relaunch)"
            ));
        }
        let mut cb = Clipboard::new()?;
        let saved = cb.get_text().ok();
        cb.set_text(text.to_string())?;
        std::thread::sleep(Duration::from_millis(60));
        post_cmd_v();
        std::thread::sleep(Duration::from_millis(150));
        if let Some(prev) = saved {
            let _ = cb.set_text(prev);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
pub use imp::{
    copy_selection, input_monitoring_granted, is_trusted, microphone_granted, paste_text,
    prompt_accessibility, request_input_monitoring,
};

#[cfg(not(target_os = "macos"))]
pub fn copy_selection() -> anyhow::Result<Option<String>> {
    Ok(None)
}

#[cfg(not(target_os = "macos"))]
pub fn paste_text(_text: &str) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn is_trusted() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn prompt_accessibility() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn microphone_granted() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn input_monitoring_granted() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn request_input_monitoring() -> bool {
    true
}
