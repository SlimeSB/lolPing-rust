use std::ffi::c_void;
use std::ptr::null;

/// The Accessibility page of System Settings, which is where the grant lives.
const PANE: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> u8;
    static kAXTrustedCheckOptionPrompt: *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    fn CFRelease(cf: *const c_void);
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    static kCFBooleanTrue: *const c_void;
}

pub fn input_access() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Adds the app to the Accessibility list and shows the system prompt: the grant can't be asked for any other way.
pub fn request_input_access() {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let options = CFDictionaryCreate(
            null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        );
        if options.is_null() {
            return;
        }
        AXIsProcessTrustedWithOptions(options);
        CFRelease(options);
    }
}

pub fn open_input_settings() {
    let _ = std::process::Command::new("open").arg(PANE).spawn();
}
