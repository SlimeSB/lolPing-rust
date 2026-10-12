use std::io;
use std::path::PathBuf;

const ENTRY_NAME: &str = "lolPing.desktop";

/// Writes or removes the autostart entry the session reads when the user logs in.
pub fn set(enabled: bool) -> io::Result<()> {
    let path = entry_path();
    if !enabled {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }
    let exe = std::env::current_exe()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, entry(&exe.to_string_lossy()))
}

fn entry_path() -> PathBuf {
    config_home().join("autostart").join(ENTRY_NAME)
}

fn config_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".config"),
        None => std::env::temp_dir(),
    }
}

fn entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=lolPing\n\
         Comment=League of Legends style ping wheel\n\
         Exec=\"{exe}\" --hidden\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}
