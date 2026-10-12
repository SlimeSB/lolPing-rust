mod capture;
mod ui;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::{json, Map, Value};

use crate::app::system_locale;
use crate::i18n::{resolve_lang, strings, Strings};
use crate::keys::UNSAFE_TRIGGER_VKS;
use crate::room::manager::RoomState;
use crate::settings::model::{self, patch_clashes, EmoteTrigger, Settings};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HelperState {
    Running,
    Failed,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub enabled: bool,
    pub helper: HelperState,
    pub first_run: bool,
    pub problems: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct DisplayView {
    pub number: u32,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

#[derive(Clone, Debug)]
pub struct RoomView {
    pub state: Arc<RoomState>,
    pub displays: Vec<DisplayView>,
}

#[derive(Debug)]
pub enum UiEvent {
    Patch(Map<String, Value>),
    ImportEmote { bytes: Vec<u8>, file_name: String },
    RemoveEmote(String),
    PreviewPing(String),
    PreviewEmote(String),
    Capturing(bool),
    RetryHelper,
    ToggleEnabled(bool),
    RoomCreate,
    RoomJoin(String),
    RoomLeave,
    RoomMute { peer: String, on: bool },
    RoomCopyCode,
    RoomRetryInternet,
    RoomRetryLan,
    RoomJoinClipboard,
    Quit,
    Closed,
}

pub enum WindowMsg {
    Status(Status),
    Settings(Box<Settings>),
    Focus,
    Close,
    Import(Result<String, String>),
    Room(Arc<RoomView>),
    /// The join the user just asked for did not even reach a room: show it under the join card.
    RoomError(String),
}

pub struct SettingsWindow {
    tx: Sender<WindowMsg>,
    alive: Arc<AtomicBool>,
}

struct Request {
    settings: Settings,
    status: Status,
    events: Sender<UiEvent>,
    messages: Receiver<WindowMsg>,
    alive: Arc<AtomicBool>,
}

impl SettingsWindow {
    pub fn open(events: Sender<UiEvent>, initial: Settings, status: Status) -> Self {
        let (tx, rx) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let request = Request { settings: initial, status, events, messages: rx, alive: Arc::clone(&alive) };
        if HOST.send(request).is_err() {
            alive.store(false, Ordering::SeqCst);
        }
        Self { tx, alive }
    }

    pub fn send(&self, msg: WindowMsg) {
        let _ = self.tx.send(msg);
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }
}

// winit allows one event loop per process and eframe caches it per thread, so every window runs on this one thread.
static HOST: LazyLock<Sender<Request>> = LazyLock::new(|| {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("lolping-settings".to_string())
        .spawn(move || host_loop(rx))
        .expect("spawn the settings window thread");
    tx
});

fn host_loop(requests: Receiver<Request>) {
    while let Ok(request) = requests.recv() {
        let app = App::new(request.events.clone(), request.messages, request.settings, request.status);
        let options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("lolPing")
                .with_inner_size([880.0, 720.0])
                .with_min_inner_size([560.0, 420.0]),
            // winit refuses an event loop off the main thread unless this is set.
            event_loop_builder: Some(Box::new(|builder| {
                winit::platform::windows::EventLoopBuilderExtWindows::with_any_thread(builder, true);
            })),
            ..Default::default()
        };
        let creator: eframe::AppCreator<'_> = Box::new(move |cc| {
            install_fonts(&cc.egui_ctx);
            Ok(Box::new(app) as Box<dyn eframe::App>)
        });
        if let Err(error) = eframe::run_native("lolPing", options, creator) {
            eprintln!("settings window: {error:?}");
        }
        request.alive.store(false, Ordering::SeqCst);
        let _ = request.events.send(UiEvent::Closed);
    }
}

const POLL: Duration = Duration::from_millis(100);

/// egui ships Latin-only faces: without one of these, every Chinese label is a row of boxes.
fn install_fonts(ctx: &egui::Context) {
    const CANDIDATES: [(&str, u32); 4] = [("Deng.ttf", 0), ("simhei.ttf", 0), ("msyh.ttc", 0), ("simsun.ttc", 0)];
    let dir = crate::paths::system_fonts();
    let Some((bytes, index)) = CANDIDATES.iter().find_map(|(name, index)| {
        std::fs::read(dir.join(name)).ok().map(|bytes| (bytes, *index))
    }) else {
        return;
    };
    let mut data = egui::FontData::from_owned(bytes);
    data.index = index;
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("system-cjk".to_owned(), std::sync::Arc::new(data));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("system-cjk".to_owned());
    }
    ctx.set_fonts(fonts);
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Target {
    Trigger,
    Emote,
    Toggle,
}

#[derive(Clone, Copy)]
pub(super) struct Capture {
    target: Target,
    hint: Option<capture::Hint>,
}

impl Capture {
    // The keys the reference's TriggerPicker forbids: the unsafe ones, plus the other key of the pair.
    fn forbidden(self, settings: &Settings) -> Vec<u32> {
        if self.target == Target::Toggle {
            return Vec::new();
        }
        let other = match self.target {
            Target::Trigger => settings.emote_trigger,
            _ => EmoteTrigger::Key(settings.trigger),
        };
        let mut vks = UNSAFE_TRIGGER_VKS.to_vec();
        if let Some(vk) = model::trigger_vk(other) {
            vks.push(vk);
        }
        vks
    }
}

struct App {
    events: Sender<UiEvent>,
    rx: Receiver<WindowMsg>,
    settings: Settings,
    status: Status,
    locale: String,
    note: Option<String>,
    import: Option<String>,
    name_draft: String,
    name_editing: bool,
    room: Option<Arc<RoomView>>,
    join_code: String,
    join_error: Option<String>,
    copied_until: Option<Instant>,
    capture: Option<Capture>,
    super_down: bool,
    focus: bool,
    closing: bool,
}

impl App {
    fn new(events: Sender<UiEvent>, rx: Receiver<WindowMsg>, settings: Settings, status: Status) -> Self {
        Self {
            events,
            rx,
            name_draft: settings.display_name.clone(),
            settings,
            status,
            locale: system_locale(),
            note: None,
            import: None,
            name_editing: false,
            room: None,
            join_code: String::new(),
            join_error: None,
            copied_until: None,
            capture: None,
            super_down: false,
            focus: false,
            closing: false,
        }
    }

    pub(super) fn t(&self) -> &'static Strings {
        strings(resolve_lang(self.settings.language, &self.locale))
    }

    pub(super) fn edit(&mut self, key: &str, value: Value) {
        self.send_patch(patch_of(key, value));
    }

    pub(super) fn send_patch(&mut self, patch: Map<String, Value>) {
        if patch_clashes(&self.settings, &patch) {
            let message = self.t().emote_key_clash.to_string();
            self.note = Some(message);
            return;
        }
        let _ = self.events.send(UiEvent::Patch(patch));
    }

    pub(super) fn start_capture(&mut self, target: Target) {
        self.stop_capture();
        self.note = None;
        self.capture = Some(Capture { target, hint: None });
        let _ = self.events.send(UiEvent::Capturing(true));
    }

    pub(super) fn stop_capture(&mut self) {
        if self.capture.take().is_some() {
            let _ = self.events.send(UiEvent::Capturing(false));
        }
    }

    fn poll(&mut self) {
        let messages: Vec<WindowMsg> = self.rx.try_iter().collect();
        for msg in messages {
            match msg {
                WindowMsg::Status(status) => self.status = status,
                WindowMsg::Settings(settings) => {
                    if !self.name_editing {
                        self.name_draft = settings.display_name.clone();
                    }
                    self.settings = *settings;
                    self.note = None;
                }
                WindowMsg::Focus => self.focus = true,
                WindowMsg::Close => {
                    self.stop_capture();
                    self.closing = true;
                }
                WindowMsg::Import(Ok(_)) => self.import = None,
                WindowMsg::Import(Err(message)) => self.import = Some(message),
                WindowMsg::Room(view) => self.room = Some(view),
                WindowMsg::RoomError(message) => self.join_error = Some(message),
            }
        }
    }

    fn read_input(&mut self, ctx: &egui::Context) {
        if self.capture.is_some() && !ctx.input(|i| i.focused) {
            self.stop_capture();
        }
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            let egui::Event::Key { key, physical_key, pressed, repeat, modifiers } = event else {
                continue;
            };
            let key = physical_key.unwrap_or(key);
            if matches!(key, egui::Key::SuperLeft | egui::Key::SuperRight) {
                self.super_down = pressed;
            }
            if pressed {
                self.on_key(key, repeat, capture::mods_of(modifiers, self.super_down));
            }
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        if !dropped.is_empty() {
            self.import = None;
            for file in dropped {
                let name = file.path().file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                match file.bytes() {
                    Ok(bytes) => self.take_import(bytes, name),
                    Err(_) => {
                        let message = self.t().import_err_disk(&name);
                        self.import = Some(message);
                    }
                }
            }
        }
    }

    fn on_key(&mut self, key: egui::Key, repeat: bool, mods: u32) {
        let Some(capture) = self.capture else {
            return;
        };
        let forbidden = capture.forbidden(&self.settings);
        let require_modifier = capture.target == Target::Toggle;
        let code = capture::code_of(key).unwrap_or("");
        match capture::evaluate(code, repeat, mods, &forbidden, require_modifier) {
            capture::Outcome::Ignore => {}
            capture::Outcome::Cancel => self.stop_capture(),
            capture::Outcome::Hint(hint) => {
                if let Some(active) = self.capture.as_mut() {
                    active.hint = Some(hint);
                }
            }
            capture::Outcome::Capture { mods, vk } => {
                let patch = match capture.target {
                    Target::Toggle => patch_of("toggleHotkey", json!({ "mods": mods, "vk": vk })),
                    Target::Trigger => patch_of("trigger", json!({ "vk": vk })),
                    Target::Emote => patch_of("emoteTrigger", json!({ "vk": vk })),
                };
                self.stop_capture();
                self.send_patch(patch);
            }
        }
    }

    pub(super) fn import_file(&mut self) {
        let picked = rfd::FileDialog::new()
            .add_filter("PNG, JPG, GIF, WebP", &["png", "jpg", "jpeg", "gif", "webp"])
            .pick_file();
        let Some(path) = picked else {
            return;
        };
        self.import = None;
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read(&path) {
            Ok(bytes) => self.take_import(bytes, file_name),
            Err(_) => {
                let message = self.t().import_err_disk(&file_name);
                self.import = Some(message);
            }
        }
    }

    // The reference's renderer refuses an oversized file itself; everything else is the app's to check.
    fn take_import(&mut self, bytes: Vec<u8>, file_name: String) {
        if bytes.len() > crate::emotes::INPUT_MAX_BYTES {
            let message = self.t().import_err_size(&file_name);
            self.import = Some(message);
            return;
        }
        let _ = self.events.send(UiEvent::ImportEmote { bytes, file_name });
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        self.read_input(ctx);
        if self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.focus {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.focus = false;
        }
        ctx.request_repaint_after(POLL);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        ui::draw(self, ui);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.capture.is_some() {
            let _ = self.events.send(UiEvent::Capturing(false));
        }
    }
}

pub(super) fn patch_of(key: &str, value: Value) -> Map<String, Value> {
    let mut patch = Map::new();
    patch.insert(key.to_string(), value);
    patch
}

// Keeps a wheel valid: an id already on another slot swaps, it never lands twice.
pub(super) fn place_on_slot(wheel: &[String], id: &str, slot: usize) -> Vec<String> {
    let mut next = wheel.to_vec();
    match next.iter().position(|held| held == id) {
        Some(from) => next.swap(from, slot),
        None => next[slot] = id.to_string(),
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emotes::DEFAULT_EMOTE_WHEEL;

    fn wheel() -> Vec<String> {
        DEFAULT_EMOTE_WHEEL.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn placing_an_id_already_on_the_wheel_swaps_the_two_slots() {
        let wheel = wheel();
        let moved = wheel[3].clone();
        let next = place_on_slot(&wheel, &moved, 0);
        assert_eq!(next[0], moved);
        assert_eq!(next[3], wheel[0]);
        assert_eq!(next.iter().collect::<std::collections::HashSet<_>>().len(), next.len());
    }

    #[test]
    fn placing_a_new_id_overwrites_the_slot() {
        let wheel = wheel();
        let next = place_on_slot(&wheel, "facepalm", 5);
        assert_eq!(next[5], "facepalm");
        assert_eq!(next[0], wheel[0]);
    }

    #[test]
    fn a_patch_names_its_field() {
        let patch = patch_of("muted", json!(true));
        assert_eq!(patch.get("muted"), Some(&json!(true)));
    }
}
