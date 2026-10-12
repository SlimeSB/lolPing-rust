use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
#[cfg(windows)]
use windows::Win32::Globalization::GetUserDefaultLocaleName;

use crate::audio::Audio;
use crate::autostart;
use crate::clipboard;
use crate::display;
use crate::emotes::library::{self, EmoteLibrary};
use crate::emotes::{self, CustomEmote};
use crate::i18n::{self, Lang, Strings};
use crate::input::decision::{Config, Emit, EmitKind, Trigger, WheelKind as HookWheel};
use crate::input::hook::{self, HookCommand, HookHandle};
use crate::keys;
use crate::overlay::scene::{Tag, Visuals, WheelKind};
use crate::overlay::{self, Command, Event as OverlayEvent};
use crate::paths::AppPaths;
use crate::permissions;
use crate::pings;
use crate::room::code::{self, ParsedRoomCode};
use crate::room::manager::{self, Profile, Room, RoomCommand, RoomEvent, RoomState};
use crate::room::relay;
use crate::settings::model::{self, EmoteTrigger, Identity, NamedTrigger, Settings, TriggerKey};
use crate::settings::store::{SettingsStore, DEFAULT_DEBOUNCE_MS};
use crate::settings_window::{DisplayView, HelperState, RoomView, SettingsWindow, Status, UiEvent, WindowMsg};
use crate::tray::{Tray, TrayCommand, TrayMode, TrayRoom};

/// The owner key of this machine's own emote on an overlay.
const SELF: &str = "self";

/// How often macOS is asked whether input access has been granted.
const ACCESS_POLL: Duration = Duration::from_millis(1500);

pub struct App {
    paths: AppPaths,
    library: EmoteLibrary,
    store: SettingsStore,
    settings: Settings,
    lang: Lang,
    enabled: bool,
    capturing: bool,
    pub quitting: bool,
    missing: Vec<String>,
    overlay: overlay::Overlay,
    overlay_events: Receiver<OverlayEvent>,
    hook: Option<HookHandle>,
    hook_tx: Sender<Emit>,
    hook_events: Receiver<Emit>,
    access_denied: bool,
    next_access_check: Instant,
    audio: Option<Audio>,
    tray: Option<Tray>,
    window: Option<SettingsWindow>,
    ui_tx: Sender<UiEvent>,
    ui_events: Receiver<UiEvent>,
    room: Option<Room>,
    room_events: Receiver<RoomEvent>,
    room_state: Option<std::sync::Arc<RoomState>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    pub fn new() -> Self {
        let paths = AppPaths::resolve();
        let identity = Identity { name: os_user_name(), color: random_tag_color() };
        let mut store = SettingsStore::new(paths.dir.clone(), DEFAULT_DEBOUNCE_MS, Some(identity));
        let mut settings = store.load().clone();
        let library = EmoteLibrary::new(paths.custom_emotes.clone());
        // An imported emote whose still is gone is dropped; normalising the shorter list repairs the wheel slots that used it.
        let kept = library.present(&settings.custom_emotes);
        if kept.len() != settings.custom_emotes.len() {
            settings = store.update(&custom_emotes_patch(kept)).clone();
        }

        let lang = i18n::resolve_lang(settings.language, &system_locale());
        let (overlay_tx, overlay_events) = std::sync::mpsc::channel();
        let overlay = overlay::Overlay::start(paths.overlay(), overlay_tx).expect("overlay windows");
        let (hook_tx, hook_events) = std::sync::mpsc::channel();
        let access_denied = !permissions::input_access();
        let hook = if access_denied { None } else { spawn_hook(&settings, settings.enabled_on_start, hook_tx.clone()) };
        let audio = Audio::new(&paths.sounds, &paths.bundled_emotes);
        let tray = Tray::new(&paths.textures.join("generic_ping.png"), i18n::strings(lang));
        let (ui_tx, ui_events) = std::sync::mpsc::channel();
        let (room_tx, room_events) = std::sync::mpsc::channel();
        let room = Room::start(
            profile_of(&settings),
            env!("CARGO_PKG_VERSION").to_string(),
            settings.allow_internet,
            relay::RELAYS.iter().map(|url| (*url).to_string()).collect(),
            room_tx,
        );

        let mut app = Self {
            library,
            settings,
            lang,
            enabled: true,
            capturing: false,
            quitting: false,
            missing: missing_assets(&paths),
            overlay,
            overlay_events,
            hook,
            hook_tx,
            hook_events,
            access_denied,
            next_access_check: Instant::now(),
            audio,
            tray,
            window: None,
            ui_tx,
            ui_events,
            room: Some(room),
            room_events,
            room_state: None,
            store,
            paths,
        };
        app.enabled = app.settings.enabled_on_start;
        app.overlay.send(Command::Visuals(Box::new(visuals_of(&app.settings))));
        if let Some(audio) = &mut app.audio {
            audio.set_muted(app.settings.muted);
        }
        app.push_status();
        app.sync_room_flags();
        if app.settings.rejoin_room {
            if let Some(code) = app.settings.last_room_code.clone() {
                app.room_send(RoomCommand::Join(code));
            }
        }
        app
    }

    pub fn ui_sender(&self) -> Sender<UiEvent> {
        self.ui_tx.clone()
    }

    pub fn open_settings(&mut self) {
        match &self.window {
            Some(window) if window.alive() => window.send(WindowMsg::Focus),
            _ => {
                self.window = Some(SettingsWindow::open(self.ui_tx.clone(), self.settings.clone(), self.status()));
            }
        }
    }

    pub fn tick(&mut self) {
        self.poll_hook();
        self.poll_access();
        self.poll_overlay();
        self.poll_room();
        self.poll_window();
        self.poll_tray();
    }

    pub fn shutdown(&mut self) {
        self.quitting = true;
        if let Some(hook) = self.hook.take() {
            hook.shutdown();
        }
        if let Some(room) = self.room.take() {
            room.shutdown();
        }
        self.store.flush();
    }

    /// After a sleep or a network change: rebind the LAN socket and poke the relays.
    pub fn wake(&mut self) {
        self.room_send(RoomCommand::Wake);
    }

    fn text(&self) -> &'static Strings {
        i18n::strings(self.lang)
    }

    fn poll_hook(&mut self) {
        let mut events = Vec::new();
        while let Ok(event) = self.hook_events.try_recv() {
            events.push(event);
        }
        for event in events {
            match event.kind {
                EmitKind::WheelOpen => {
                    let maps = display::displays();
                    let Some(local) = display::physical_to_local(event.x, event.y, &maps, None) else {
                        continue;
                    };
                    let kind = wheel_kind(event.wheel);
                    self.overlay.send(Command::OpenWheel { display_id: local.display_id, kind, x: event.x, y: event.y });
                }
                EmitKind::WheelMove => self.overlay.send(Command::MoveWheel { x: event.x, y: event.y }),
                EmitKind::WheelRelease => self.overlay.send(Command::ReleaseWheel { x: event.x, y: event.y }),
                EmitKind::Click => self.click(event),
                EmitKind::Cancel => self.overlay.send(Command::CancelWheel),
                EmitKind::Toggled => self.set_enabled(event.enabled),
            }
        }
    }

    /// A trigger + click: the ping or emote the wheel's centre stands for, sent without the wheel.
    fn click(&mut self, event: Emit) {
        let maps = display::displays();
        let Some(local) = display::physical_to_local(event.x, event.y, &maps, None) else {
            return;
        };
        match event.wheel {
            HookWheel::Ping => {
                let ping = self.settings.click_ping_id.to_string();
                self.overlay.send(Command::SpawnPing {
                    display_id: local.display_id,
                    ping: ping.clone(),
                    x: event.x,
                    y: event.y,
                    tag: None,
                });
                self.play_ping(&ping);
                self.share_ping_at(local.display_id, event.x, event.y, &ping);
            }
            HookWheel::Emote => {
                let reference = self.settings.click_emote_id.clone();
                let art = emotes::resolve_emote_art(
                    &reference,
                    &self.settings.custom_emotes,
                    false,
                    &self.paths.bundled_emotes,
                    &self.paths.custom_emotes,
                );
                self.overlay.send(Command::SpawnEmote {
                    display_id: local.display_id,
                    art,
                    owner: SELF.to_string(),
                    x: event.x,
                    y: event.y,
                    tag: None,
                });
                self.play_emote(&reference);
                self.share_emote_at(local.display_id, event.x, event.y, &reference);
            }
        }
    }

    fn poll_overlay(&mut self) {
        let mut events = Vec::new();
        while let Ok(event) = self.overlay_events.try_recv() {
            events.push(event);
        }
        for event in events {
            match event {
                OverlayEvent::Hover(Some(_)) => self.tick_sound(),
                OverlayEvent::Hover(None) | OverlayEvent::Cancelled => {}
                OverlayEvent::Released { kind, key, display_id, x, y } => match kind {
                    WheelKind::Ping => {
                        self.play_ping(&key);
                        self.share_ping(display_id, f64::from(x), f64::from(y), &key);
                    }
                    WheelKind::Emote => {
                        self.play_emote(&key);
                        self.share_emote(display_id, f64::from(x), f64::from(y), &key);
                    }
                },
            }
        }
    }

    fn poll_window(&mut self) {
        let mut events = Vec::new();
        while let Ok(event) = self.ui_events.try_recv() {
            events.push(event);
        }
        let mut closed = false;
        for event in events {
            match event {
                UiEvent::Patch(patch) => self.apply_patch(patch),
                UiEvent::ImportEmote { bytes, file_name } => self.import_emote(&bytes, &file_name),
                UiEvent::RemoveEmote(id) => self.remove_emote(&id),
                UiEvent::PreviewPing(id) => self.preview_ping(&id),
                UiEvent::PreviewEmote(reference) => self.preview_emote(&reference),
                UiEvent::Capturing(on) => self.set_capturing(on),
                UiEvent::RetryHelper => self.restart_hook(),
                UiEvent::OpenAccessibility => self.open_accessibility(),
                UiEvent::ToggleEnabled(on) => self.set_enabled(on),
                UiEvent::RoomCreate => self.room_send(RoomCommand::Create),
                UiEvent::RoomJoin(text) => self.join_room(&text),
                UiEvent::RoomJoinClipboard => self.join_from_clipboard(),
                UiEvent::RoomLeave => self.room_send(RoomCommand::Leave),
                UiEvent::RoomMute { peer, on } => self.room_send(RoomCommand::Mute { peer, on }),
                UiEvent::RoomCopyCode => self.copy_room_code(),
                UiEvent::RoomRetryInternet => self.room_send(RoomCommand::RetryInternet),
                UiEvent::RoomRetryLan => self.room_send(RoomCommand::RetryLan),
                UiEvent::Quit => self.quitting = true,
                UiEvent::Closed => closed = true,
            }
        }
        if closed {
            self.window = None;
        }
    }

    fn poll_tray(&mut self) {
        loop {
            let Some(command) = self.tray.as_ref().and_then(Tray::poll) else {
                return;
            };
            match command {
                TrayCommand::OpenSettings | TrayCommand::RoomOpenSettings => self.open_settings(),
                TrayCommand::SetEnabled(on) => self.set_enabled(on),
                TrayCommand::RetryHelper => self.restart_hook(),
                TrayCommand::Quit => self.quitting = true,
                TrayCommand::RoomJoinClipboard => self.join_from_clipboard(),
                TrayCommand::RoomCreate => self.room_send(RoomCommand::Create),
                TrayCommand::RoomCopyCode => self.copy_room_code(),
                TrayCommand::RoomSetMuted(on) => {
                    let mut patch = Map::new();
                    patch.insert("roomMuted".to_string(), Value::Bool(on));
                    self.apply_patch(patch);
                }
                TrayCommand::RoomLeave => self.room_send(RoomCommand::Leave),
            }
        }
    }

    fn poll_room(&mut self) {
        let mut events = Vec::new();
        while let Ok(event) = self.room_events.try_recv() {
            events.push(event);
        }
        for event in events {
            match event {
                RoomEvent::State(state) => {
                    self.room_state = Some(state);
                    self.push_room();
                }
                RoomEvent::Ping { number, fx, fy, id, tag } => self.remote_ping(number, fx, fy, &id, tag),
                RoomEvent::Emote { number, fx, fy, reference, peer, tag } => {
                    self.remote_emote(number, fx, fy, &reference, &peer, tag);
                }
                RoomEvent::Toast(toast) => self.room_toast(toast),
                RoomEvent::SaveCode(code) => {
                    let mut patch = Map::new();
                    patch.insert("lastRoomCode".to_string(), code.map(Value::String).unwrap_or(Value::Null));
                    self.apply_patch(patch);
                }
            }
        }
    }

    fn room_send(&self, command: RoomCommand) {
        if let Some(room) = &self.room {
            room.send(command);
        }
    }

    /// The room needs to know when pings must be dropped and when the room is muted for us.
    fn sync_room_flags(&self) {
        self.room_send(RoomCommand::SetFlags { paused: !self.enabled, room_muted: self.settings.room_muted });
    }

    fn push_room(&mut self) {
        let Some(state) = self.room_state.clone() else {
            return;
        };
        if let Some(tray) = &mut self.tray {
            let room = TrayRoom { code: state.code.clone(), count: state.members.len(), muted: self.settings.room_muted };
            tray.set_room(room);
        }
        if let Some(window) = &self.window {
            if window.alive() {
                let view = RoomView { state, displays: display_views() };
                window.send(WindowMsg::Room(std::sync::Arc::new(view)));
            }
        }
    }

    /// A room mate's ping: their display number and a fraction of its size, mapped onto ours.
    fn remote_ping(&mut self, number: u32, fx: f64, fy: f64, id: &str, tag: manager::Tag) {
        let Some((display_id, x, y)) = remote_point(number, fx, fy) else {
            return;
        };
        self.overlay.send(Command::SpawnPing {
            display_id,
            ping: id.to_string(),
            x,
            y,
            tag: Some(scene_tag(tag)),
        });
        self.play_ping(id);
    }

    fn remote_emote(&mut self, number: u32, fx: f64, fy: f64, reference: &str, peer: &str, tag: manager::Tag) {
        if !emotes::is_emote_ref_shape(reference) {
            return;
        }
        let Some((display_id, x, y)) = remote_point(number, fx, fy) else {
            return;
        };
        let art = emotes::resolve_emote_art(
            reference,
            &self.settings.custom_emotes,
            true,
            &self.paths.bundled_emotes,
            &self.paths.custom_emotes,
        );
        self.overlay.send(Command::SpawnEmote {
            display_id,
            art,
            owner: peer.to_string(),
            x,
            y,
            tag: Some(scene_tag(tag)),
        });
        // A member has one emote at a time, whichever display the earlier one was on.
        self.overlay.send(Command::ClearEmote { owner: peer.to_string(), except_display: Some(display_id) });
        self.play_emote(reference);
    }

    fn room_toast(&mut self, toast: manager::Toast) {
        let t = self.text();
        let code = self.room_state.as_ref().and_then(|state| state.code.clone()).unwrap_or_default();
        let (title, body) = match toast {
            manager::Toast::JoinedRoom { code } => (t.toast_room_joined(&code), t.toast_room_joined_body.to_string()),
            manager::Toast::Full => (t.toast_room_full.to_string(), t.toast_room_full_body.to_string()),
            manager::Toast::Joined { name } => (t.toast_member_joined(&name), code),
            manager::Toast::Left { name } => (t.toast_member_left(&name), code),
            manager::Toast::Lost { name } => (t.toast_member_lost(&name), code),
        };
        self.toast(&title, &body);
    }

    fn join_room(&mut self, text: &str) {
        match code::parse_room_code(text, false) {
            ParsedRoomCode::Valid(code) => self.room_send(RoomCommand::Join(code)),
            ParsedRoomCode::Invalid => self.join_failed(self.text().room_invalid),
            ParsedRoomCode::Mistyped => self.join_failed(self.text().room_mistyped),
        }
    }

    fn join_failed(&self, message: &str) {
        if let Some(window) = &self.window {
            if window.alive() {
                window.send(WindowMsg::RoomError(message.to_string()));
            }
        }
    }

    /// Nobody typed this one, so only an unmistakable code counts.
    fn join_from_clipboard(&mut self) {
        let text = clipboard::read_text().unwrap_or_default();
        match code::parse_room_code(&text, true) {
            ParsedRoomCode::Valid(code) => self.room_send(RoomCommand::Join(code)),
            _ => {
                let t = self.text();
                self.toast(t.toast_no_code, t.toast_no_code_body);
            }
        }
    }

    fn copy_room_code(&self) {
        let Some(code) = self.room_state.as_ref().and_then(|state| state.code.clone()) else {
            return;
        };
        clipboard::write_text(&code);
    }

    /// Our own ping, for the room: the display number and a fraction of its size.
    fn share_ping(&self, display_id: u32, x: f64, y: f64, id: &str) {
        let Some((number, fx, fy)) = display::to_shared(&numbered(), display_id, x, y) else {
            return;
        };
        self.room_send(RoomCommand::SendPing { number, fx, fy, id: id.to_string() });
    }

    fn share_ping_at(&self, display_id: u32, x: i32, y: i32, id: &str) {
        let maps = display::displays();
        let Some(local) = display::physical_to_local(x, y, &maps, Some(display_id)) else {
            return;
        };
        self.share_ping(local.display_id, local.x, local.y, id);
    }

    fn share_emote(&self, display_id: u32, x: f64, y: f64, reference: &str) {
        let Some((number, fx, fy)) = display::to_shared(&numbered(), display_id, x, y) else {
            return;
        };
        self.room_send(RoomCommand::SendEmote { number, fx, fy, reference: reference.to_string() });
    }

    fn share_emote_at(&self, display_id: u32, x: i32, y: i32, reference: &str) {
        let maps = display::displays();
        let Some(local) = display::physical_to_local(x, y, &maps, Some(display_id)) else {
            return;
        };
        self.share_emote(local.display_id, local.x, local.y, reference);
    }

    fn apply_patch(&mut self, patch: Map<String, Value>) {
        if model::patch_clashes(&self.settings, &patch) {
            return;
        }
        let next = self.store.update(&patch).clone();
        self.apply_settings(next);
    }

    fn apply_settings(&mut self, next: Settings) {
        let previous = std::mem::replace(&mut self.settings, next);
        self.send_hook_config();
        self.overlay.send(Command::Visuals(Box::new(visuals_of(&self.settings))));
        if let Some(audio) = &mut self.audio {
            audio.set_muted(self.settings.muted);
        }
        if self.settings.language != previous.language {
            self.lang = i18n::resolve_lang(self.settings.language, &system_locale());
            let text = self.text();
            if let Some(tray) = &mut self.tray {
                tray.set_text(text);
            }
        }
        if self.settings.launch_at_startup != previous.launch_at_startup {
            if let Err(err) = autostart::set(self.settings.launch_at_startup) {
                eprintln!("autostart: {err}");
            }
        }
        if self.settings.display_name != previous.display_name
            || self.settings.tag_color != previous.tag_color
            || self.settings.incoming_ping_limit != previous.incoming_ping_limit
        {
            self.room_send(RoomCommand::Profile(profile_of(&self.settings)));
        }
        if self.settings.room_muted != previous.room_muted {
            self.sync_room_flags();
        }
        if self.settings.allow_internet != previous.allow_internet {
            self.room_send(RoomCommand::SetInternet(self.settings.allow_internet));
        }
        self.push_settings();
        self.push_status();
    }

    fn set_enabled(&mut self, on: bool) {
        if on == self.enabled {
            return;
        }
        self.enabled = on;
        self.send_hook_config();
        self.sync_room_flags();
        let t = self.text();
        let hint = t.toast_toggle_hint(&keys::hotkey_label(self.settings.toggle_hotkey));
        let title = if on { t.toast_on } else { t.toast_off };
        self.toast(title, &hint);
        self.push_status();
    }

    fn set_capturing(&mut self, on: bool) {
        if on == self.capturing {
            return;
        }
        self.capturing = on;
        if let Some(hook) = &self.hook {
            hook.send(HookCommand::Suspend(on));
        }
    }

    fn restart_hook(&mut self) {
        if let Some(hook) = self.hook.take() {
            hook.shutdown();
        }
        self.hook = spawn_hook(&self.settings, self.enabled, self.hook_tx.clone());
        if self.capturing {
            if let Some(hook) = &self.hook {
                hook.send(HookCommand::Suspend(true));
            }
        }
        self.push_status();
    }

    /// The helper reads input only where the OS allows it, and macOS waits for the grant before it starts.
    fn helper_state(&self) -> HelperState {
        if self.hook.is_some() {
            return HelperState::Running;
        }
        if self.access_denied || permissions::GATED && permissions::input_access() {
            return HelperState::NoAccess { stale: !self.access_denied };
        }
        HelperState::Failed
    }

    /// macOS: start the helper as soon as the user grants input access.
    fn poll_access(&mut self) {
        if self.quitting || self.hook.is_some() || !self.access_denied {
            return;
        }
        if Instant::now() < self.next_access_check {
            return;
        }
        self.next_access_check = Instant::now() + ACCESS_POLL;
        if permissions::input_access() {
            self.access_denied = false;
            self.restart_hook();
        }
    }

    /// Asks for input access and opens the page where it is granted: macOS only lists the app once it asks.
    fn open_accessibility(&mut self) {
        permissions::request_input_access();
        permissions::open_input_settings();
    }

    fn import_emote(&mut self, bytes: &[u8], file_name: &str) {
        let file = match library::prepare_emote(bytes, file_name) {
            Ok(file) => file,
            Err(error) => {
                self.import_failed(file_name, error);
                return;
            }
        };
        match self.library.save(&file, &self.settings.custom_emotes) {
            Ok(emote) => {
                let mut list = self.settings.custom_emotes.clone();
                list.push(emote.clone());
                let next = self.store.update(&custom_emotes_patch(list)).clone();
                self.apply_settings(next);
                if let Some(window) = &self.window {
                    window.send(WindowMsg::Import(Ok(emote.id)));
                }
            }
            Err(error) => self.import_failed(file_name, error),
        }
    }

    fn import_failed(&self, file_name: &str, error: emotes::ImportError) {
        let t = self.text();
        let message = match error {
            emotes::ImportError::Type => t.import_err_type(file_name),
            emotes::ImportError::Size => t.import_err_size(file_name),
            emotes::ImportError::AnimSize => t.import_err_anim_size(file_name),
            emotes::ImportError::Detail => t.import_err_detail(file_name),
            emotes::ImportError::Duplicate => t.import_err_duplicate(file_name),
            emotes::ImportError::Full => t.import_err_full.to_string(),
            emotes::ImportError::Disk => t.import_err_disk(file_name),
        };
        if let Some(window) = &self.window {
            window.send(WindowMsg::Import(Err(message)));
        }
    }

    fn remove_emote(&mut self, id: &str) {
        if !emotes::is_custom_ref(id) {
            return;
        }
        self.library.remove(id);
        let kept: Vec<CustomEmote> = self.settings.custom_emotes.iter().filter(|c| c.id != id).cloned().collect();
        let next = self.store.update(&custom_emotes_patch(kept)).clone();
        self.apply_settings(next);
    }

    fn preview_ping(&mut self, id: &str) {
        let Some(def) = pings::ping_by_id(id) else {
            return;
        };
        let Some(primary) = primary_display() else {
            return;
        };
        self.overlay.send(Command::SpawnPing {
            display_id: primary.id,
            ping: def.id.to_string(),
            x: primary.phys.x + primary.phys.width / 2,
            y: primary.phys.y + primary.phys.height / 2,
            tag: None,
        });
        self.play_ping(def.id);
    }

    fn preview_emote(&mut self, reference: &str) {
        if !emotes::is_emote_ref_shape(reference) {
            return;
        }
        let Some(primary) = primary_display() else {
            return;
        };
        let art = emotes::resolve_emote_art(
            reference,
            &self.settings.custom_emotes,
            false,
            &self.paths.bundled_emotes,
            &self.paths.custom_emotes,
        );
        self.overlay.send(Command::SpawnEmote {
            display_id: primary.id,
            art,
            owner: "preview".to_string(),
            x: primary.phys.x + primary.phys.width / 2,
            y: primary.phys.y + primary.phys.height / 2,
            tag: None,
        });
    }

    fn toast(&self, title: &str, body: &str) {
        let Some(primary) = primary_display() else {
            return;
        };
        self.overlay.send(Command::Toast { display_id: primary.id, title: title.to_string(), body: body.to_string() });
    }

    fn volume(&self) -> f32 {
        if self.settings.muted {
            0.0
        } else {
            self.settings.volume as f32 / 100.0
        }
    }

    fn tick_sound(&mut self) {
        if !self.settings.tick_sound {
            return;
        }
        let volume = self.volume() * 0.5;
        if let Some(audio) = &mut self.audio {
            audio.play(pings::TICK_SOUND, volume);
        }
    }

    fn play_ping(&mut self, id: &str) {
        let Some(def) = pings::ping_by_id(id) else {
            return;
        };
        let volume = self.volume();
        if let Some(audio) = &mut self.audio {
            audio.play(def.sound, volume);
        }
    }

    fn play_emote(&mut self, reference: &str) {
        if !self.settings.emote_sound || !emotes::is_bundled(reference) {
            return;
        }
        let Some(entry) = emotes::catalog::by_slug(reference) else {
            return;
        };
        if !entry.has_sound {
            return;
        }
        let name = format!("emote:{}", entry.slug);
        let volume = self.volume();
        if let Some(audio) = &mut self.audio {
            audio.play(&name, volume);
        }
    }

    fn send_hook_config(&self) {
        if let Some(hook) = &self.hook {
            hook.send(HookCommand::Config(hook_config(&self.settings, self.enabled)));
        }
    }

    fn status(&self) -> Status {
        Status {
            enabled: self.enabled,
            helper: self.helper_state(),
            first_run: self.store.first_run,
            problems: self.problems(),
        }
    }

    fn problems(&self) -> Vec<String> {
        let t = self.text();
        let mut out = self.store.problems();
        if self.store.emote_key_clashed {
            out.push(t.problem_emote_key_clash.to_string());
        }
        for asset in &self.missing {
            out.push(t.missing_asset(asset));
        }
        out
    }

    fn push_status(&mut self) {
        let mode = match self.helper_state() {
            HelperState::NoAccess { .. } => TrayMode::NoAccess,
            HelperState::Failed => TrayMode::Failed,
            HelperState::Running if self.enabled => TrayMode::On,
            HelperState::Running => TrayMode::Off,
        };
        if let Some(tray) = &mut self.tray {
            tray.set_status(mode, self.enabled);
        }
        if let Some(window) = &self.window {
            if window.alive() {
                window.send(WindowMsg::Status(self.status()));
            }
        }
    }

    fn push_settings(&self) {
        if let Some(window) = &self.window {
            if window.alive() {
                window.send(WindowMsg::Settings(Box::new(self.settings.clone())));
            }
        }
    }
}

fn primary_display() -> Option<display::DisplayMap> {
    let maps = display::displays();
    maps.iter().find(|m| m.primary).or_else(|| maps.first()).copied()
}

fn profile_of(settings: &Settings) -> Profile {
    Profile {
        name: settings.display_name.clone(),
        color: settings.tag_color,
        limit: settings.incoming_ping_limit,
    }
}

fn scene_tag(tag: manager::Tag) -> Tag {
    Tag { name: tag.name, color: tag.color as usize }
}

/// The displays in the order room pings number them.
fn numbered() -> Vec<display::NumberedDisplay> {
    let maps = display::displays();
    display::number_displays(&maps.iter().map(|m| (m.id, m.dip, m.primary)).collect::<Vec<_>>(), primary_id(&maps))
}

fn primary_id(maps: &[display::DisplayMap]) -> u32 {
    maps.iter().find(|m| m.primary).map(|m| m.id).unwrap_or(0)
}

fn display_views() -> Vec<DisplayView> {
    let maps = display::displays();
    let list = display::number_displays(&maps.iter().map(|m| (m.id, m.dip, m.primary)).collect::<Vec<_>>(), primary_id(&maps));
    list.iter()
        .filter_map(|numbered| {
            let map = maps.iter().find(|m| m.id == numbered.id)?;
            Some(DisplayView {
                number: numbered.number,
                width: map.phys.width,
                height: map.phys.height,
                primary: numbered.primary,
            })
        })
        .collect()
}

/// A room mate's ping or emote: their display number and fraction, as physical pixels here.
fn remote_point(number: u32, fx: f64, fy: f64) -> Option<(u32, i32, i32)> {
    let maps = display::displays();
    let target = *display::resolve_target(&numbered(), number)?;
    let x = fx * f64::from(target.bounds.width);
    let y = fy * f64::from(target.bounds.height);
    let (x, y) = display::local_to_physical(&maps, target.id, x, y)?;
    Some((target.id, x, y))
}

fn custom_emotes_patch(list: Vec<CustomEmote>) -> Map<String, Value> {
    let mut patch = Map::new();
    patch.insert("customEmotes".to_string(), serde_json::to_value(list).unwrap_or(Value::Null));
    patch
}

/// Every asset the overlays load, by its place in assets/: what is missing is listed in the settings window.
fn missing_assets(paths: &AppPaths) -> Vec<String> {
    let mut missing = Vec::new();
    for name in pings::all_texture_names() {
        let file = format!("textures/{name}.png");
        if !paths.assets.join(&file).is_file() {
            missing.push(file);
        }
    }
    for name in pings::all_sound_names() {
        let file = format!("sounds/{name}.wav");
        if !paths.assets.join(&file).is_file() {
            missing.push(file);
        }
    }
    for file in emotes::all_emote_asset_files() {
        if !paths.assets.join(&file).is_file() {
            missing.push(file);
        }
    }
    missing
}

fn visuals_of(settings: &Settings) -> Visuals {
    let o = model::overlay_settings(settings);
    Visuals {
        ping_size_px: o.ping_size_px as f32,
        ping_duration_s: o.ping_duration_s as f32,
        emote_size_px: o.emote_size_px as f32,
        wheel: o.wheel.iter().map(|id| (*id).to_string()).collect(),
        click_ping: o.click_ping_id.to_string(),
        emote_wheel: o.emote_wheel.clone(),
        click_emote: o.click_emote_id.clone(),
        custom_emotes: o.custom_emotes.clone(),
        tick_sound: o.tick_sound,
        emote_sound: o.emote_sound,
    }
}

fn hook_config(settings: &Settings, enabled: bool) -> Config {
    Config {
        trigger: trigger_of(settings.trigger),
        trigger_vk: match settings.trigger {
            TriggerKey::Vk { vk } => vk,
            TriggerKey::Named(_) => 0,
        },
        click_ping: settings.click_ping,
        game_check: settings.game_check,
        emote_trigger: match settings.emote_trigger {
            EmoteTrigger::Off => Trigger::None,
            EmoteTrigger::Key(key) => trigger_of(key),
        },
        emote_trigger_vk: model::trigger_vk(settings.emote_trigger).unwrap_or(0),
        emote_click: settings.emote_click,
        drag_threshold_px: settings.drag_threshold_px as i32,
        toggle_mods: settings.toggle_hotkey.mods,
        toggle_vk: settings.toggle_hotkey.vk,
        enabled,
    }
}

fn trigger_of(key: TriggerKey) -> Trigger {
    match key {
        TriggerKey::Vk { .. } => Trigger::CustomVk,
        TriggerKey::Named(NamedTrigger::Alt) => Trigger::Alt,
        TriggerKey::Named(NamedTrigger::Ctrl) => Trigger::Ctrl,
        TriggerKey::Named(NamedTrigger::Shift) => Trigger::Shift,
        TriggerKey::Named(NamedTrigger::Win) => Trigger::Win,
        TriggerKey::Named(NamedTrigger::Capslock) => Trigger::CapsLock,
        TriggerKey::Named(NamedTrigger::Mouse4) => Trigger::Mouse4,
        TriggerKey::Named(NamedTrigger::Mouse5) => Trigger::Mouse5,
    }
}

fn wheel_kind(kind: HookWheel) -> WheelKind {
    match kind {
        HookWheel::Ping => WheelKind::Ping,
        HookWheel::Emote => WheelKind::Emote,
    }
}

fn spawn_hook(settings: &Settings, enabled: bool, sink: Sender<Emit>) -> Option<HookHandle> {
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let handle = match hook::spawn(sink, ready_tx) {
        Ok(handle) => handle,
        Err(err) => {
            eprintln!("input hook: {err}");
            return None;
        }
    };
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(_)) => {}
        Ok(Err(message)) => {
            eprintln!("input hook: {message}");
            return None;
        }
        Err(_) => {
            eprintln!("input hook: no answer");
            return None;
        }
    }
    handle.send(HookCommand::Config(hook_config(settings, enabled)));
    Some(handle)
}

fn os_user_name() -> String {
    std::env::var("USERNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Player".to_string())
}

/// This machine's default tag colour: one of the eight room colours.
fn random_tag_color() -> u32 {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    nanos % 8
}

#[cfg(windows)]
pub(crate) fn system_locale() -> String {
    let mut buffer = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
    if len <= 1 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..(len - 1) as usize])
}

#[cfg(target_os = "linux")]
pub(crate) fn system_locale() -> String {
    let raw = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_MESSAGES"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    raw.split(['.', '@']).next().unwrap_or_default().replace('_', "-")
}

#[cfg(target_os = "macos")]
pub(crate) fn system_locale() -> String {
    objc2_foundation::NSLocale::currentLocale().localeIdentifier().to_string().replace('_', "-")
}
