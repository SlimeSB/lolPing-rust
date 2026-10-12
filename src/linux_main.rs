use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use lolping::app::App;

const POLL: Duration = Duration::from_millis(16);
static OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn main() {
    let hidden = std::env::args().any(|arg| arg == "--hidden");
    let path = socket_path();
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            // The socket file outlives a crash, so only a listener that answers means another instance.
            if ask(&path) {
                return;
            }
            let _ = std::fs::remove_file(&path);
            match UnixListener::bind(&path) {
                Ok(listener) => listener,
                Err(_) => return,
            }
        }
        Err(_) => return,
    };
    std::thread::Builder::new()
        .name("lolping-instance".to_string())
        .spawn(move || {
            for stream in listener.incoming() {
                if stream.is_err() {
                    break;
                }
                OPEN_REQUESTED.store(true, Ordering::Relaxed);
            }
        })
        .expect("spawn the instance listener");
    // The tray is a GTK widget, so this thread has to be the one driving the GTK loop.
    if gtk::init().is_err() {
        eprintln!("lolPing: GTK did not start, so there is no tray icon");
    }
    let mut app = App::new();
    if !hidden {
        app.open_settings();
    }
    loop {
        if OPEN_REQUESTED.swap(false, Ordering::Relaxed) {
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

fn ask(path: &Path) -> bool {
    UnixStream::connect(path)
        .map(|mut stream| stream.write_all(b"open").is_ok())
        .unwrap_or(false)
}

fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join("lolPing.sock")
}
