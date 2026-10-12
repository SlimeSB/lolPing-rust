use std::ffi::c_void;
use std::time::Duration;

use lolping::app::App;
use lolping::settings_window;

use crate::instance;

const FRAME: Duration = Duration::from_millis(16);

struct Tick {
    app: *mut App,
    open_once: bool,
}

pub fn main() {
    let hidden = std::env::args().any(|arg| arg == "--hidden");
    if !instance::acquire() {
        return;
    }
    // AppKit wants the tray and the event tap's windows on the main thread, so this thread keeps the app.
    let app = Box::into_raw(Box::new(App::new()));
    let tick = Box::into_raw(Box::new(Tick { app, open_once: !hidden || !lolping::permissions::input_access() }));
    install_tick(tick.cast());
    settings_window::mac_run(unsafe { (*app).ui_sender() });
}

/// A run loop timer keeps the app ticking while the settings window is hidden and gets no redraws.
fn install_tick(info: *mut c_void) {
    let context = Box::into_raw(Box::new(objc2_core_foundation::CFRunLoopTimerContext {
        version: 0,
        info,
        retain: None,
        release: None,
        copyDescription: None,
    }));
    let timer = unsafe {
        objc2_core_foundation::CFRunLoopTimer::new(
            None,
            objc2_core_foundation::CFAbsoluteTimeGetCurrent() + 0.5,
            FRAME.as_secs_f64(),
            0,
            0,
            Some(on_tick),
            context,
        )
    };
    let Some(loop_) = objc2_core_foundation::CFRunLoop::current() else {
        return;
    };
    if let Some(timer) = &timer {
        objc2_core_foundation::CFRunLoop::add_timer(&loop_, Some(timer), unsafe {
            objc2_core_foundation::kCFRunLoopCommonModes
        });
    }
}

unsafe extern "C-unwind" fn on_tick(_timer: *mut objc2_core_foundation::CFRunLoopTimer, info: *mut c_void) {
    let tick = unsafe { &mut *info.cast::<Tick>() };
    let app = unsafe { &mut *tick.app };
    if tick.open_once || instance::take_open_request() {
        tick.open_once = false;
        app.open_settings();
    }
    lolping::overlay::window::pump();
    app.tick();
    if app.quitting {
        app.shutdown();
        std::process::exit(0);
    }
}
