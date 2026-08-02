//! Read ~200 characters of text around the caret via Accessibility, for cleanup
//! context awareness. Soft-fails to `None` when AX is unavailable, the field is
//! secure, or the app doesn't expose a text value.

#[cfg(target_os = "macos")]
mod imp {
    use std::os::raw::{c_char, c_void};
    use std::ptr;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type AXUIElementRef = *const c_void;

    const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    /// `kAXValueCFRangeType` — location/length selection range.
    const K_AX_VALUE_CF_RANGE_TYPE: u32 = 4;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CFRange {
        location: isize,
        length: isize,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CFStringCreateWithCString(
            alloc: CFTypeRef,
            cstr: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFStringGetLength(s: CFStringRef) -> isize;
        fn CFStringGetCString(s: CFStringRef, buf: *mut c_char, size: isize, encoding: u32) -> bool;
        fn CFStringGetMaximumSizeForEncoding(len: isize, encoding: u32) -> isize;
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFStringCompare(a: CFStringRef, b: CFStringRef, options: usize) -> isize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(boolean: CFTypeRef) -> bool;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: CFStringRef,
            value: *mut CFTypeRef,
        ) -> i32;
        fn AXValueGetValue(value: CFTypeRef, type_id: u32, value_ptr: *mut c_void) -> bool;
    }

    fn make_cfstring(s: &str) -> CFStringRef {
        let Ok(c) = std::ffi::CString::new(s) else {
            return ptr::null();
        };
        unsafe { CFStringCreateWithCString(ptr::null(), c.as_ptr(), KCF_STRING_ENCODING_UTF8) }
    }

    unsafe fn cfstring_to_string(s: CFStringRef) -> Option<String> {
        if s.is_null() || CFGetTypeID(s) != CFStringGetTypeID() {
            return None;
        }
        let len = CFStringGetLength(s);
        let max = CFStringGetMaximumSizeForEncoding(len, KCF_STRING_ENCODING_UTF8) + 1;
        if max <= 0 {
            return Some(String::new());
        }
        let mut buf = vec![0i8; max as usize];
        if CFStringGetCString(s, buf.as_mut_ptr(), max, KCF_STRING_ENCODING_UTF8) {
            std::ffi::CStr::from_ptr(buf.as_ptr())
                .to_str()
                .ok()
                .map(|x| x.to_string())
        } else {
            None
        }
    }

    unsafe fn copy_attribute(element: AXUIElementRef, name: &str) -> CFTypeRef {
        let attr = make_cfstring(name);
        if attr.is_null() {
            return ptr::null();
        }
        let mut value: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(element, attr, &mut value);
        CFRelease(attr);
        if err != 0 {
            ptr::null()
        } else {
            value
        }
    }

    unsafe fn copy_focused_element() -> AXUIElementRef {
        let system = AXUIElementCreateSystemWide();
        if system.is_null() {
            return ptr::null();
        }
        let focused = copy_attribute(system, "AXFocusedUIElement");
        CFRelease(system);
        focused as AXUIElementRef
    }

    unsafe fn element_string(element: AXUIElementRef, name: &str) -> Option<String> {
        let value = copy_attribute(element, name);
        if value.is_null() {
            return None;
        }
        let s = cfstring_to_string(value as CFStringRef);
        CFRelease(value);
        s
    }

    unsafe fn string_attribute_equals(element: AXUIElementRef, name: &str, expected: &str) -> bool {
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

    unsafe fn bool_attribute(element: AXUIElementRef, name: &str) -> Option<bool> {
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

    fn is_secure_field(element: AXUIElementRef) -> bool {
        unsafe {
            string_attribute_equals(element, "AXSubrole", "AXSecureTextField")
                || string_attribute_equals(element, "AXRole", "AXSecureTextField")
                || bool_attribute(element, "AXProtectedContent") == Some(true)
        }
    }

    unsafe fn selected_range(element: AXUIElementRef) -> Option<CFRange> {
        let value = copy_attribute(element, "AXSelectedTextRange");
        if value.is_null() {
            return None;
        }
        let mut range = CFRange {
            location: 0,
            length: 0,
        };
        let ok = AXValueGetValue(
            value,
            K_AX_VALUE_CF_RANGE_TYPE,
            &mut range as *mut CFRange as *mut c_void,
        );
        CFRelease(value);
        if ok {
            Some(range)
        } else {
            None
        }
    }

    /// Slice ~`chars_before` / ~`chars_after` around the caret (UTF-16 indices from AX).
    pub fn read_caret_context(chars_before: usize, chars_after: usize) -> Option<String> {
        if !crate::paste::is_trusted() {
            return None;
        }
        unsafe {
            let element = copy_focused_element();
            if element.is_null() {
                return None;
            }
            if is_secure_field(element) {
                CFRelease(element);
                return None;
            }
            let Some(text) = element_string(element, "AXValue") else {
                CFRelease(element);
                return None;
            };
            if text.trim().is_empty() {
                CFRelease(element);
                return None;
            }

            // AX ranges are UTF-16 code units; approximate with char indices for slicing.
            let chars: Vec<char> = text.chars().collect();
            let len = chars.len();
            let caret = selected_range(element)
                .map(|r| {
                    let loc = r.location.max(0) as usize;
                    // Prefer the end of the selection so we read around the insertion point.
                    loc.saturating_add(r.length.max(0) as usize)
                })
                .unwrap_or(len)
                .min(len);
            CFRelease(element);

            let start = caret.saturating_sub(chars_before);
            let end = (caret + chars_after).min(len);
            if start >= end {
                return None;
            }
            let slice: String = chars[start..end].iter().collect();
            let trimmed = slice.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
    }
}

/// Read ~200 characters around the caret. Returns `None` off macOS or on soft failure.
pub fn read_caret_context(chars_before: usize, chars_after: usize) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        imp::read_caret_context(chars_before, chars_after)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (chars_before, chars_after);
        None
    }
}
