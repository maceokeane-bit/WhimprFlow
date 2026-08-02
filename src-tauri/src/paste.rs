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
    use std::thread;
    use std::time::Duration;

    type CGEventRef = *mut c_void;
    type CGEventSourceRef = *const c_void;
    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type AXUIElementRef = *const c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreateKeyboardEvent(
            source: CGEventSourceRef,
            keycode: u16,
            keydown: bool,
        ) -> CGEventRef;
        fn CGEventSetFlags(event: CGEventRef, flags: u64);
        fn CGEventKeyboardSetUnicodeString(
            event: CGEventRef,
            string_length: usize,
            unicode_string: *const u16,
        );
        fn CGEventPost(tap: u32, event: CGEventRef);
        fn CGPreflightListenEventAccess() -> bool;
        fn CGRequestListenEventAccess() -> bool;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFStringGetLength(string: CFStringRef) -> isize;
        fn CFStringCreateWithCString(
            alloc: CFTypeRef,
            cstr: *const i8,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringCreateWithCharacters(
            alloc: CFTypeRef,
            chars: *const u16,
            num_chars: isize,
        ) -> CFStringRef;
        fn CFStringCompare(a: CFStringRef, b: CFStringRef, options: usize) -> isize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(boolean: CFTypeRef) -> bool;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXUIElementIsAttributeSettable(
            element: AXUIElementRef,
            attribute: CFStringRef,
            settable: *mut bool,
        ) -> i32;
        fn AXUIElementSetAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: CFTypeRef,
        ) -> i32;
    }

    const KCG_HID_EVENT_TAP: u32 = 0;
    const KCG_FLAG_MASK_COMMAND: u64 = 0x0010_0000;
    const KEYCODE_V: u16 = 9;
    const KEYCODE_C: u16 = 8;
    const UTF8_ENCODING: u32 = 0x0800_0100;
    const CHUNK_CHARS: usize = 250;

    struct FocusedElement(AXUIElementRef);

    impl Drop for FocusedElement {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) };
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum InsertionPolicy {
        AxPreferred,
        ChunkedClipboard,
    }

    fn policy_for_bundle(bundle_id: Option<&str>) -> InsertionPolicy {
        match bundle_id.unwrap_or_default() {
            "com.apple.Terminal"
            | "com.googlecode.iterm2"
            | "dev.warp.Warp-Stable"
            | "com.mitchellh.ghostty"
            | "com.microsoft.VSCode"
            | "com.todesktop.230313mzl4w4u92" => InsertionPolicy::ChunkedClipboard,
            _ => InsertionPolicy::AxPreferred,
        }
    }

    fn make_cfstring(value: &str) -> CFStringRef {
        let Ok(value) = std::ffi::CString::new(value) else {
            return null();
        };
        unsafe { CFStringCreateWithCString(null(), value.as_ptr(), UTF8_ENCODING) }
    }

    unsafe fn copy_attribute(element: AXUIElementRef, name: &str) -> CFTypeRef {
        let attribute = make_cfstring(name);
        if attribute.is_null() {
            return null();
        }
        let mut value = null();
        let result = AXUIElementCopyAttributeValue(element, attribute, &mut value);
        CFRelease(attribute);
        if result == 0 {
            value
        } else {
            null()
        }
    }

    fn focused_element() -> Option<FocusedElement> {
        unsafe {
            let system = AXUIElementCreateSystemWide();
            if system.is_null() {
                return None;
            }
            let focused = copy_attribute(system, "AXFocusedUIElement");
            CFRelease(system);
            if focused.is_null() {
                None
            } else {
                Some(FocusedElement(focused))
            }
        }
    }

    fn string_attribute_equals(element: AXUIElementRef, name: &str, expected: &str) -> bool {
        unsafe {
            let value = copy_attribute(element, name);
            if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
                if !value.is_null() {
                    CFRelease(value);
                }
                return false;
            }
            let expected = make_cfstring(expected);
            if expected.is_null() {
                CFRelease(value);
                return false;
            }
            let equal = CFStringCompare(value as CFStringRef, expected, 0) == 0;
            CFRelease(expected);
            CFRelease(value);
            equal
        }
    }

    fn bool_attribute(element: AXUIElementRef, name: &str) -> Option<bool> {
        unsafe {
            let value = copy_attribute(element, name);
            if value.is_null() || CFGetTypeID(value) != CFBooleanGetTypeID() {
                if !value.is_null() {
                    CFRelease(value);
                }
                return None;
            }
            let result = CFBooleanGetValue(value);
            CFRelease(value);
            Some(result)
        }
    }

    fn is_secure_field(element: AXUIElementRef) -> bool {
        string_attribute_equals(element, "AXSubrole", "AXSecureTextField")
            || string_attribute_equals(element, "AXRole", "AXSecureTextField")
            || bool_attribute(element, "AXProtectedContent") == Some(true)
    }

    fn ax_insert(element: AXUIElementRef, text: &str) -> bool {
        unsafe {
            let attribute = make_cfstring("AXSelectedText");
            if attribute.is_null() {
                return false;
            }
            let mut settable = false;
            let can_set =
                AXUIElementIsAttributeSettable(element, attribute, &mut settable) == 0 && settable;
            if !can_set {
                CFRelease(attribute);
                return false;
            }
            let utf16: Vec<u16> = text.encode_utf16().collect();
            let value = CFStringCreateWithCharacters(null(), utf16.as_ptr(), utf16.len() as isize);
            if value.is_null() {
                CFRelease(attribute);
                return false;
            }
            let inserted = AXUIElementSetAttributeValue(element, attribute, value) == 0;
            CFRelease(value);
            CFRelease(attribute);
            inserted
        }
    }

    fn string_attribute_length(element: AXUIElementRef, attribute: &str) -> Option<isize> {
        unsafe {
            let value = copy_attribute(element, attribute);
            if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
                if !value.is_null() {
                    CFRelease(value);
                }
                return None;
            }
            let length = CFStringGetLength(value as CFStringRef);
            CFRelease(value);
            Some(length)
        }
    }

    fn value_length(element: AXUIElementRef) -> Option<isize> {
        string_attribute_length(element, "AXValue")
    }

    fn text_chunks(text: &str, max_chars: usize) -> Vec<&str> {
        if text.is_empty() {
            return vec![];
        }
        let mut chunks = Vec::new();
        let mut start = 0;
        let mut chars = 0;
        for (index, _) in text.char_indices() {
            if chars == max_chars {
                chunks.push(&text[start..index]);
                start = index;
                chars = 0;
            }
            chars += 1;
        }
        chunks.push(&text[start..]);
        chunks
    }

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
            if !down.is_null() {
                CGEventSetFlags(down, KCG_FLAG_MASK_COMMAND);
                CGEventPost(KCG_HID_EVENT_TAP, down);
                CFRelease(down);
            }

            let up = CGEventCreateKeyboardEvent(null(), keycode, false);
            if !up.is_null() {
                CGEventSetFlags(up, KCG_FLAG_MASK_COMMAND);
                CGEventPost(KCG_HID_EVENT_TAP, up);
                CFRelease(up);
            }
        }
    }

    fn post_cmd_v() {
        post_cmd(KEYCODE_V);
    }

    fn post_cmd_c() {
        post_cmd(KEYCODE_C);
    }

    fn type_unicode(text: &str) -> anyhow::Result<()> {
        for chunk in text_chunks(text, 32) {
            let utf16: Vec<u16> = chunk.encode_utf16().collect();
            unsafe {
                let down = CGEventCreateKeyboardEvent(null(), 0, true);
                let up = CGEventCreateKeyboardEvent(null(), 0, false);
                if down.is_null() || up.is_null() {
                    if !down.is_null() {
                        CFRelease(down);
                    }
                    if !up.is_null() {
                        CFRelease(up);
                    }
                    return Err(anyhow::anyhow!("could not create Unicode keyboard event"));
                }
                CGEventKeyboardSetUnicodeString(down, utf16.len(), utf16.as_ptr());
                CGEventPost(KCG_HID_EVENT_TAP, down);
                CGEventPost(KCG_HID_EVENT_TAP, up);
                CFRelease(down);
                CFRelease(up);
            }
            thread::sleep(Duration::from_millis(4));
        }
        Ok(())
    }

    /// Returns `Some(false)` only when AX can prove a collapsed insertion
    /// point did not change. `None` means the target cannot be verified, so a
    /// retry would risk inserting duplicate text.
    fn clipboard_insert(
        clipboard: &mut arboard::Clipboard,
        text: &str,
        chunked: bool,
        element: Option<AXUIElementRef>,
    ) -> anyhow::Result<Option<bool>> {
        let original = clipboard.get_text().ok();
        let before = element.and_then(value_length);
        let selected =
            element.and_then(|element| string_attribute_length(element, "AXSelectedText"));
        let chunks = if chunked {
            text_chunks(text, CHUNK_CHARS)
        } else {
            vec![text]
        };

        for chunk in chunks {
            clipboard.set_text(chunk.to_string())?;
            thread::sleep(Duration::from_millis(20));
            post_cmd_v();
            thread::sleep(Duration::from_millis(if chunked { 75 } else { 35 }));
        }

        thread::sleep(Duration::from_millis(180));
        let after = element.and_then(value_length);
        match original {
            Some(value) => {
                let _ = clipboard.set_text(value);
            }
            None => {
                let _ = clipboard.clear();
            }
        }

        match (before, after, selected) {
            (Some(before), Some(after), Some(0)) => Ok(Some(before != after || text.is_empty())),
            (Some(_), Some(_), Some(_)) => Ok(Some(true)),
            _ => Ok(None),
        }
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
        if text.is_empty() {
            return Ok(());
        }
        if !is_trusted() {
            return Err(anyhow::anyhow!(
                "no Accessibility permission — cannot paste (grant it in System Settings → \
                 Privacy & Security → Accessibility, then relaunch)"
            ));
        }

        let element = focused_element();
        if let Some(element) = element.as_ref() {
            if is_secure_field(element.0) {
                return Err(anyhow::anyhow!(
                    "dictation is disabled in password and secure text fields"
                ));
            }
        }

        let bundle_id = crate::appctx::frontmost_bundle_id();
        let policy = policy_for_bundle(bundle_id.as_deref());
        if policy == InsertionPolicy::AxPreferred {
            if let Some(element) = element.as_ref() {
                if ax_insert(element.0, text) {
                    return Ok(());
                }
            }
        }

        let mut cb = Clipboard::new()?;
        let verified = clipboard_insert(
            &mut cb,
            text,
            policy == InsertionPolicy::ChunkedClipboard,
            element.as_ref().map(|element| element.0),
        )?;
        if verified == Some(false) {
            type_unicode(text)?;
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn chunks_on_character_boundaries() {
            let text = "a".repeat(249) + "🙂b";
            let chunks = text_chunks(&text, 250);
            assert_eq!(chunks.len(), 2);
            assert_eq!(chunks[0].chars().count(), 250);
            assert_eq!(chunks[1], "b");
        }

        #[test]
        fn applies_chunking_overrides() {
            assert_eq!(
                policy_for_bundle(Some("com.apple.Terminal")),
                InsertionPolicy::ChunkedClipboard
            );
            assert_eq!(
                policy_for_bundle(Some("com.example.Editor")),
                InsertionPolicy::AxPreferred
            );
        }
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
