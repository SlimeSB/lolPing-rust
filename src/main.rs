#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
mod linux_main;
#[cfg(windows)]
mod win_main;

fn main() {
    #[cfg(target_os = "linux")]
    linux_main::main();
    #[cfg(windows)]
    win_main::main();
}
