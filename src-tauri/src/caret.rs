//! Frontmost-window geometry — used to pick the correct monitor for the pill.

#[derive(Debug, Clone, Copy)]
pub struct ScreenRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

pub fn rect_center(rect: ScreenRect) -> (f64, f64) {
    (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0)
}

pub fn frontmost_window_frame() -> Option<ScreenRect> {
    #[cfg(target_os = "macos")]
    {
        imp::frontmost_window_frame()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::ScreenRect;
    use std::os::raw::{c_char, c_void};
    use std::ptr;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;
    type AXUIElementRef = *const c_void;

    const K_AX_VALUE_CGRECT_TYPE: u32 = 5;

    #[repr(C)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    struct CGSize {
        width: f64,
        height: f64,
    }

    #[repr(C)]
    struct CGRect {
        origin: CGPoint,
        size: CGSize,
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: CFTypeRef);
        fn CFStringCreateWithCString(
            alloc: CFTypeRef,
            cstr: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
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
        unsafe { CFStringCreateWithCString(ptr::null(), c.as_ptr(), 0x0800_0100) }
    }

    unsafe fn copy_attribute(element: AXUIElementRef, name: &str) -> CFTypeRef {
        let attr = make_cfstring(name);
        let mut value: CFTypeRef = ptr::null();
        let err = AXUIElementCopyAttributeValue(element, attr, &mut value);
        if !attr.is_null() {
            CFRelease(attr);
        }
        if err != 0 {
            return ptr::null();
        }
        value
    }

    unsafe fn element_frame(element: AXUIElementRef) -> Option<ScreenRect> {
        let frame = copy_attribute(element, "AXFrame");
        if frame.is_null() {
            return None;
        }
        let mut rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: 0.0,
                height: 0.0,
            },
        };
        if !AXValueGetValue(
            frame,
            K_AX_VALUE_CGRECT_TYPE,
            &mut rect as *mut CGRect as *mut c_void,
        ) {
            CFRelease(frame);
            return None;
        }
        CFRelease(frame);
        if rect.size.width < 1.0 || rect.size.height < 1.0 {
            return None;
        }
        Some(ScreenRect {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        })
    }

    pub fn frontmost_window_frame() -> Option<ScreenRect> {
        unsafe {
            let Some(pid) = crate::appctx::frontmost_pid() else {
                return None;
            };
            let app = AXUIElementCreateApplication(pid);
            if app.is_null() {
                return None;
            }
            for name in ["AXFocusedWindow", "AXMainWindow"] {
                let win = copy_attribute(app, name);
                if win.is_null() {
                    continue;
                }
                let frame = element_frame(win as AXUIElementRef);
                CFRelease(app);
                CFRelease(win);
                return frame;
            }
            CFRelease(app);
            None
        }
    }
}
