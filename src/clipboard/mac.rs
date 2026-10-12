use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
use objc2_foundation::NSString;

/// The general pasteboard outlives every call, so nothing has to be kept alive here.
pub fn read_text() -> Option<String> {
    let pasteboard = NSPasteboard::generalPasteboard();
    let text = pasteboard.stringForType(unsafe { NSPasteboardTypeString })?;
    Some(text.to_string())
}

pub fn write_text(text: &str) -> bool {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString })
}
