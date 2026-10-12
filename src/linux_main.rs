use std::time::Duration;

use lolping::app::App;

use crate::instance;

const POLL: Duration = Duration::from_millis(16);

pub fn main() {
    let hidden = std::env::args().any(|arg| arg == "--hidden");
    if !instance::acquire() {
        return;
    }
    // The tray is a GTK widget, so this thread has to be the one driving the GTK loop.
    if gtk::init().is_err() {
        eprintln!("lolPing: GTK did not start, so there is no tray icon");
    }
    let mut app = App::new();
    if !hidden {
        app.open_settings();
    }
    loop {
        if instance::take_open_request() {
            app.open_settings();
        }
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        app.tick();
        if app.quitting {
            break;
        }
        std::thread::sleep(POLL);
    }
    app.shutdown();
}
