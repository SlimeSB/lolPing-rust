use std::sync::atomic::{AtomicBool, Ordering};

use lolping::app::App;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowW, MsgWaitForMultipleObjectsEx, PeekMessageW,
    PostMessageW, PostQuitMessage, RegisterClassW, TranslateMessage, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT,
    WM_APP, WM_CLOSE, WM_DESTROY, WM_QUIT, WNDCLASSW, WS_OVERLAPPED,
};

/// Only ever found by the second instance, which posts to it to open the settings window.
const WINDOW_CLASS: &str = "lolPing";
const OPEN_SETTINGS: u32 = WM_APP + 1;
/// WM_POWERBROADCAST with PBT_APMRESUMEAUTOMATIC: the machine slept, so nobody was heard while we weren't listening.
const POWER_BROADCAST: u32 = 0x0218;
const APM_RESUME_AUTOMATIC: usize = 0x0012;
static OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);
static RESUMED: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        OPEN_SETTINGS => {
            OPEN_REQUESTED.store(true, Ordering::Relaxed);
            LRESULT(0)
        }
        POWER_BROADCAST if wparam.0 == APM_RESUME_AUTOMATIC => {
            RESUMED.store(true, Ordering::Relaxed);
            LRESULT(1)
        }
        WM_CLOSE => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

pub fn main() {
    let hidden = std::env::args().any(|arg| arg == "--hidden");
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // One instance per session: a second launch only asks the running one to show its settings.
        let _lock = CreateMutexW(None, false, w!("Local\\lolPing"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let class = wide(WINDOW_CLASS);
            if let Ok(window) = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR::null()) {
                if !window.is_invalid() {
                    let _ = PostMessageW(Some(window), OPEN_SETTINGS, WPARAM(0), LPARAM(0));
                }
            }
            return;
        }
        let mut app = App::new();
        if !hidden {
            app.open_settings();
        }
        let _window = match control_window() {
            Some(window) => window,
            None => return,
        };
        let mut msg = MSG::default();
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    app.shutdown();
                    return;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            if OPEN_REQUESTED.swap(false, Ordering::Relaxed) {
                app.open_settings();
            }
            if RESUMED.swap(false, Ordering::Relaxed) {
                app.wake();
            }
            app.tick();
            if app.quitting {
                app.shutdown();
                return;
            }
            // Sleep until the next message or the next frame, whichever comes first.
            MsgWaitForMultipleObjectsEx(None, 16, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn control_window() -> Option<HWND> {
    let class = wide(WINDOW_CLASS);
    let name = wide("lolPing");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        lpszClassName: PCWSTR(class.as_ptr()),
        ..Default::default()
    };
    RegisterClassW(&wc);
    let window = CreateWindowExW(
        Default::default(),
        PCWSTR(class.as_ptr()),
        PCWSTR(name.as_ptr()),
        WS_OVERLAPPED,
        0,
        0,
        0,
        0,
        None,
        None,
        None,
        None,
    )
    .ok()?;
    Some(window)
}
