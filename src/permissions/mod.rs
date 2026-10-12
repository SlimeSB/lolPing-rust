#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;

/// Whether this platform can be waiting for a grant before the helper may read input.
pub const GATED: bool = cfg!(target_os = "macos");

#[cfg(not(target_os = "macos"))]
pub fn input_access() -> bool {
    true
}

#[cfg(not(target_os = "macos"))]
pub fn request_input_access() {}

#[cfg(not(target_os = "macos"))]
pub fn open_input_settings() {}
