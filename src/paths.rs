use std::path::PathBuf;

/// Where the app reads and writes: the data folder under %APPDATA% and the assets shipped next to the exe.
pub struct AppPaths {
    pub dir: PathBuf,
    pub assets: PathBuf,
    pub bundled_emotes: PathBuf,
    pub custom_emotes: PathBuf,
    pub sounds: PathBuf,
    pub textures: PathBuf,
    pub fonts: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Self {
        let dir = data_dir();
        let assets = assets_dir();
        Self {
            bundled_emotes: assets.join("emotes"),
            sounds: assets.join("sounds"),
            textures: assets.join("textures"),
            fonts: system_fonts(),
            custom_emotes: dir.join("emotes"),
            assets,
            dir,
        }
    }

    pub fn overlay(&self) -> crate::overlay::Paths {
        crate::overlay::Paths {
            assets: self.assets.clone(),
            bundled_emotes: self.bundled_emotes.clone(),
            custom_emotes: self.custom_emotes.clone(),
            fonts: self.fonts.clone(),
        }
    }
}

fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    base.join("lolPing")
}

/// Where Windows keeps its own faces; the overlay and the settings window both draw from here.
pub fn system_fonts() -> PathBuf {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    PathBuf::from(root).join("Fonts")
}

/// Next to the exe in a shipped build, in the source tree while developing.
fn assets_dir() -> PathBuf {
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("assets"))) {
        if dir.is_dir() {
            return dir;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}
