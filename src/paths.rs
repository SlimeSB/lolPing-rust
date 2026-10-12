use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Where the app reads and writes: the data folder under %APPDATA% and the assets shipped next to the exe.
pub struct AppPaths {
    pub dir: PathBuf,
    pub assets: PathBuf,
    pub bundled_emotes: PathBuf,
    pub custom_emotes: PathBuf,
    pub sounds: PathBuf,
    pub textures: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Self {
        let dir = data_dir();
        let assets = assets_dir();
        Self {
            bundled_emotes: assets.join("emotes"),
            sounds: assets.join("sounds"),
            textures: assets.join("textures"),
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
        }
    }
}

fn data_dir() -> PathBuf {
    data_root().join("lolPing")
}

#[cfg(windows)]
fn data_root() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir)
}

#[cfg(target_os = "linux")]
fn data_root() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(target_os = "macos")]
fn data_root() -> PathBuf {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"))
        .unwrap_or_else(std::env::temp_dir)
}

/// The faces with Chinese, in the order the overlay and the settings window both prefer.
#[cfg(windows)]
pub const CJK_FACES: [(&[&str], u32); 4] = [
    (&["Deng.ttf"], 0),
    (&["msyh.ttc"], 0),
    (&["simhei.ttf"], 0),
    (&["simsun.ttc"], 0),
];

#[cfg(target_os = "linux")]
pub const CJK_FACES: [(&[&str], u32); 4] = [
    (&["NotoSansCJK-Regular.ttc", "NotoSansCJKsc-Regular.otf"], 0),
    (&["NotoSansSC-Regular.otf", "NotoSansSC-Regular.ttf"], 0),
    (&["wqy-zenhei.ttc"], 0),
    (&["wqy-microhei.ttc", "DroidSansFallbackFull.ttf"], 0),
];

#[cfg(target_os = "macos")]
pub const CJK_FACES: [(&[&str], u32); 4] = [
    (&["PingFang.ttc"], 0),
    (&["Hiragino Sans GB.ttc"], 0),
    (&["STHeiti Light.ttc"], 0),
    (&["Songti.ttc"], 0),
];

/// The first of these faces the system has, wherever its font tree keeps them.
pub fn font_path(candidates: &[&str]) -> Option<PathBuf> {
    candidates.iter().find_map(|name| font_files().get(&name.to_ascii_lowercase()).cloned())
}

fn font_files() -> &'static HashMap<String, PathBuf> {
    static FILES: LazyLock<HashMap<String, PathBuf>> = LazyLock::new(|| {
        let mut files = HashMap::new();
        for root in font_roots() {
            collect_fonts(&root, 3, &mut files);
        }
        files
    });
    &FILES
}

fn collect_fonts(dir: &Path, depth: u32, files: &mut HashMap<String, PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                collect_fonts(&path, depth - 1, files);
            }
            continue;
        }
        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
            files.entry(name.to_ascii_lowercase()).or_insert(path);
        }
    }
}

#[cfg(windows)]
fn font_roots() -> Vec<PathBuf> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    vec![PathBuf::from(root).join("Fonts")]
}

#[cfg(target_os = "linux")]
fn font_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        roots.push(home.join(".local/share/fonts"));
        roots.push(home.join(".fonts"));
    }
    roots
}

#[cfg(target_os = "macos")]
fn font_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/System/Library/Fonts"), PathBuf::from("/Library/Fonts")];
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home).join("Library/Fonts"));
    }
    roots
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
