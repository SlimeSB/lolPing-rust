#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(unix)]
mod instance;

#[cfg(target_os = "linux")]
mod linux_main;
#[cfg(target_os = "macos")]
mod mac_main;
#[cfg(windows)]
mod win_main;

fn main() {
    #[cfg(target_os = "linux")]
    linux_main::main();
    #[cfg(target_os = "macos")]
    mac_main::main();
    #[cfg(windows)]
    win_main::main();
}
