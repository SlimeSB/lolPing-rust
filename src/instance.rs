use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

static OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Claims the single-instance socket, or asks the copy that holds it to open its settings and gives up.
pub fn acquire() -> bool {
    let path = socket_path();
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
            // The socket file outlives a crash, so only a listener that answers means another instance.
            if ask(&path) {
                return false;
            }
            let _ = std::fs::remove_file(&path);
            match UnixListener::bind(&path) {
                Ok(listener) => listener,
                Err(_) => return false,
            }
        }
        Err(_) => return false,
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
        .ok();
    true
}

pub fn take_open_request() -> bool {
    OPEN_REQUESTED.swap(false, Ordering::Relaxed)
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
