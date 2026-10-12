#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(windows)]
mod win_main;

fn main() {
    #[cfg(windows)]
    win_main::main();
}
