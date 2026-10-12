use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{Map, Value};

use crate::settings::model::{merge, normalize, EmoteTrigger, Identity, Settings};

pub const DEFAULT_DEBOUNCE_MS: u64 = 300;

/// %APPDATA%\lolPing, where Electron's userData pointed.
pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("lolPing")
}

/// Delay before each retry: 3 attempts in total.
const RENAME_RETRY_DELAYS_MS: [u64; 2] = [50, 100];

enum Msg {
    Save(Settings),
    Flush(Settings, Sender<()>),
}

/// Loads, validates and persists settings.json.
pub struct SettingsStore {
    pub file: PathBuf,
    /// True when load() found no settings file: lolPing's first run.
    pub first_run: bool,
    /// True when load() read an emote key that clashed with another lolPing key and turned it off.
    pub emote_key_clashed: bool,
    dir: PathBuf,
    current: Settings,
    identity: Identity,
    tx: Option<Sender<Msg>>,
    writer: Option<JoinHandle<()>>,
    problems: Arc<Mutex<Vec<String>>>,
}

impl SettingsStore {
    /// `identity` supplies this machine's defaults for the room name and tag colour.
    pub fn new(dir: PathBuf, debounce_ms: u64, identity: Option<Identity>) -> Self {
        let identity = identity.unwrap_or_default();
        let file = dir.join("settings.json");
        let problems = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = mpsc::channel();
        let writer = thread::spawn({
            let dir = dir.clone();
            let file = file.clone();
            let problems = Arc::clone(&problems);
            move || writer_loop(rx, dir, file, Duration::from_millis(debounce_ms), problems)
        });
        Self {
            file,
            first_run: false,
            emote_key_clashed: false,
            dir,
            current: normalize(&Value::Null, &identity),
            identity,
            tx: Some(tx),
            writer: Some(writer),
            problems,
        }
    }

    pub fn load(&mut self) -> &Settings {
        self.emote_key_clashed = false;
        self.first_run = !self.file.exists();
        if self.first_run {
            self.current = normalize(&Value::Null, &self.identity);
            self.schedule_save();
            return &self.current;
        }
        let text = match fs::read_to_string(&self.file) {
            Ok(text) => text,
            Err(err) => {
                self.restore_defaults(&err.to_string());
                return &self.current;
            }
        };
        let raw: Value = match serde_json::from_str(&text) {
            Ok(raw) => raw,
            Err(err) => {
                self.restore_defaults(&err.to_string());
                return &self.current;
            }
        };
        self.current = normalize(&raw, &self.identity);
        let stored = raw.as_object();
        let written = serde_json::to_value(&self.current).expect("Settings serializes");
        // A file from an older version lacks the newer keys: write the filled-in values once.
        let complete = written
            .as_object()
            .is_some_and(|w| w.keys().all(|k| stored.is_some_and(|s| s.contains_key(k))));
        if !complete {
            self.schedule_save();
        }
        self.emote_key_clashed = stored
            .and_then(|s| s.get("emoteTrigger"))
            .is_some_and(|v| v.as_str() != Some("off"))
            && self.current.emote_trigger == EmoteTrigger::Off;
        &self.current
    }

    pub fn get(&self) -> &Settings {
        &self.current
    }

    pub fn problems(&self) -> Vec<String> {
        self.problems.lock().clone()
    }

    pub fn update(&mut self, patch: &Map<String, Value>) -> &Settings {
        self.current = merge(&self.current, patch, &self.identity);
        self.schedule_save();
        &self.current
    }

    /// Writes now via a temp file + rename; safe to call repeatedly.
    pub fn flush(&mut self) {
        let (ack_tx, ack_rx) = mpsc::channel();
        if let Some(tx) = &self.tx {
            if tx.send(Msg::Flush(self.current.clone(), ack_tx)).is_ok() {
                let _ = ack_rx.recv();
            }
        }
    }

    fn schedule_save(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Save(self.current.clone()));
        }
    }

    fn restore_defaults(&mut self, message: &str) {
        let mut backup = "it was backed up to settings.bak.json".to_string();
        if let Err(err) = fs::copy(&self.file, self.dir.join("settings.bak.json")) {
            backup = format!("it could not be backed up to settings.bak.json ({err})");
        }
        self.problems
            .lock()
            .push(format!("settings.json could not be read ({message}); {backup} and defaults were restored."));
        self.current = normalize(&Value::Null, &self.identity);
    }
}

impl Drop for SettingsStore {
    fn drop(&mut self) {
        self.flush();
        self.tx = None;
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

fn writer_loop(rx: Receiver<Msg>, dir: PathBuf, file: PathBuf, debounce: Duration, problems: Arc<Mutex<Vec<String>>>) {
    let mut pending: Option<Settings> = None;
    let mut deadline = Instant::now();
    loop {
        let msg = if pending.is_some() {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match rx.recv() {
                Ok(msg) => Some(msg),
                Err(_) => break,
            }
        };
        match msg {
            Some(Msg::Save(settings)) => {
                pending = Some(settings);
                deadline = Instant::now() + debounce;
            }
            Some(Msg::Flush(settings, ack)) => {
                pending = Some(settings);
                write_pending(&mut pending, &dir, &file, &problems);
                let _ = ack.send(());
            }
            None => write_pending(&mut pending, &dir, &file, &problems),
        }
    }
}

fn write_pending(pending: &mut Option<Settings>, dir: &Path, file: &Path, problems: &Mutex<Vec<String>>) {
    let Some(settings) = pending.take() else {
        return;
    };
    let json = serde_json::to_string_pretty(&settings).expect("Settings serializes");
    if let Err(err) = write_atomic(dir, file, &json) {
        problems.lock().push(format!("Could not save settings: {err}"));
    }
}

fn write_atomic(dir: &Path, file: &Path, data: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut name = file.as_os_str().to_owned();
    name.push(".tmp");
    let tmp = PathBuf::from(name);
    let result = fs::write(&tmp, data).and_then(|()| rename_with_retry(&tmp, file));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Renames, retrying the brief locks Windows can hold on the target.
pub fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempt = 0;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(err) if attempt < RENAME_RETRY_DELAYS_MS.len() && transient_rename_error(&err) => {
                thread::sleep(Duration::from_millis(RENAME_RETRY_DELAYS_MS[attempt]));
                attempt += 1;
            }
            Err(err) => return Err(err),
        }
    }
}

fn transient_rename_error(err: &io::Error) -> bool {
    matches!(err.raw_os_error(), Some(1 | 5 | 13 | 32 | 33 | 170)) || err.kind() == io::ErrorKind::PermissionDenied
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::model::{NamedTrigger, TriggerKey};
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "lolping-settings-{}-{}-{}",
            std::process::id(),
            tag,
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn first_run_writes_defaults() {
        let dir = temp_dir("first");
        let mut store = SettingsStore::new(dir.clone(), 5, None);
        let loaded = store.load().clone();
        assert!(store.first_run);
        assert_eq!(loaded, Settings::default());
        store.flush();
        let text = fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(text.contains("\"pingDurationS\": 3.2"));
        assert!(!dir.join("settings.json.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_old_file_is_filled_in_and_updates_survive_a_reload() {
        let dir = temp_dir("old");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), r#"{ "volume": 40, "trigger": "mouse4" }"#).unwrap();
        let mut store = SettingsStore::new(dir.clone(), 5, None);
        let loaded = store.load().clone();
        assert!(!store.first_run);
        assert_eq!(loaded.volume, 40);
        assert_eq!(loaded.trigger, TriggerKey::Named(NamedTrigger::Mouse4));

        store.flush();
        let text = fs::read_to_string(dir.join("settings.json")).unwrap();
        assert!(text.contains("\"dragThresholdPx\": 8"));

        let mut patch = Map::new();
        patch.insert("volume".to_string(), json!(55));
        store.update(&patch);
        store.flush();

        let mut reopened = SettingsStore::new(dir.clone(), 5, None);
        assert_eq!(reopened.load().volume, 55);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_file_is_backed_up() {
        let dir = temp_dir("bad");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), "{ not json").unwrap();
        let mut store = SettingsStore::new(dir.clone(), 5, None);
        let loaded = store.load().clone();
        assert_eq!(loaded, Settings::default());
        assert_eq!(store.problems().len(), 1);
        assert_eq!(fs::read_to_string(dir.join("settings.bak.json")).unwrap(), "{ not json");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_clashing_emote_key_is_reported() {
        let dir = temp_dir("clash");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("settings.json"), r#"{ "trigger": "alt", "emoteTrigger": "alt" }"#).unwrap();
        let mut store = SettingsStore::new(dir.clone(), 5, None);
        let loaded = store.load().clone();
        assert!(store.emote_key_clashed);
        assert_eq!(loaded.emote_trigger, EmoteTrigger::Off);

        fs::write(dir.join("settings.json"), r#"{ "trigger": "alt" }"#).unwrap();
        let mut upgraded = SettingsStore::new(dir.clone(), 5, None);
        upgraded.load();
        assert!(!upgraded.emote_key_clashed);
        let _ = fs::remove_dir_all(&dir);
    }
}
