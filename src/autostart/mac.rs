use std::io;
use std::path::PathBuf;

const LABEL: &str = "com.slimesb.lolping";
const ENTRY_NAME: &str = "com.slimesb.lolping.plist";

/// Writes or removes the LaunchAgent the login session reads when the user logs in.
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
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    home.join("Library").join("LaunchAgents").join(ENTRY_NAME)
}

fn entry(exe: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         <key>Label</key>\n\
         <string>{LABEL}</string>\n\
         <key>ProgramArguments</key>\n\
         <array>\n\
         <string>{exe}</string>\n\
         <string>--hidden</string>\n\
         </array>\n\
         <key>RunAtLoad</key>\n\
         <true/>\n\
         </dict>\n\
         </plist>\n"
    )
}
