use std::path::Path;

use image::{imageops::FilterType, RgbaImage};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::Strings;

pub enum TrayCommand {
    OpenSettings,
    SetEnabled(bool),
    RetryHelper,
    Quit,
    RoomJoinClipboard,
    RoomCreate,
    RoomCopyCode,
    RoomSetMuted(bool),
    RoomLeave,
    RoomOpenSettings,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TrayMode {
    On,
    Off,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayRoom {
    pub code: Option<String>,
    pub count: usize,
    pub muted: bool,
}

struct Icons {
    on: Icon,
    off: Icon,
    failed: Icon,
}

fn tint(base: &RgbaImage, f: impl Fn(u8, u8, u8, u8) -> [u8; 4]) -> RgbaImage {
    let mut out = base.clone();
    for pixel in out.pixels_mut() {
        let [r, g, b, a] = pixel.0;
        pixel.0 = f(r, g, b, a);
    }
    out
}

fn icon(image: &RgbaImage) -> Option<Icon> {
    let small = image::imageops::resize(image, 32, 32, FilterType::Lanczos3);
    let (width, height) = (small.width(), small.height());
    Icon::from_rgba(small.into_raw(), width, height).ok()
}

fn load_icons(path: &Path) -> Option<Icons> {
    let base = image::open(path).ok()?.to_rgba8();
    let grey = tint(&base, |r, g, b, a| {
        let l = (0.3 * f32::from(r) + 0.59 * f32::from(g) + 0.11 * f32::from(b)) * 0.5;
        let l = l.clamp(0.0, 255.0) as u8;
        [l, l, l, (f32::from(a) * 0.5) as u8]
    });
    let warning = tint(&base, |_r, _g, _b, a| {
        [a, (f32::from(a) * 0.55) as u8, (f32::from(a) * 0.15) as u8, a]
    });
    Some(Icons { on: icon(&base)?, off: icon(&grey)?, failed: icon(&warning)? })
}

pub struct Tray {
    icon: TrayIcon,
    icons: Icons,
    text: &'static Strings,
    mode: TrayMode,
    enabled: bool,
    room: TrayRoom,
    toggle: Option<CheckMenuItem>,
    room_mute: Option<CheckMenuItem>,
}

impl Tray {
    pub fn new(icon_path: &Path, text: &'static Strings) -> Option<Self> {
        let icons = load_icons(icon_path)?;
        let icon = TrayIconBuilder::new()
            .with_icon(icons.on.clone())
            .with_tooltip(text.tray_tip(true))
            .with_menu_on_left_click(false)
            .with_menu(Box::new(Menu::new()))
            .build()
            .ok()?;
        let mut tray = Self { icon, icons, text, mode: TrayMode::On, enabled: true, room: TrayRoom::default(), toggle: None, room_mute: None };
        tray.render();
        Some(tray)
    }

    pub fn set_status(&mut self, mode: TrayMode, enabled: bool) {
        if self.mode == mode && self.enabled == enabled {
            return;
        }
        self.mode = mode;
        self.enabled = enabled;
        self.render();
    }

    pub fn set_room(&mut self, room: TrayRoom) {
        if self.room == room {
            return;
        }
        self.room = room;
        self.render();
    }

    pub fn set_text(&mut self, text: &'static Strings) {
        self.text = text;
        self.render();
    }

    /// Drains the shell's menu and icon events; the caller runs the message loop that delivers them.
    pub fn poll(&self) -> Option<TrayCommand> {
        let menu = MenuEvent::receiver();
        while let Ok(event) = menu.try_recv() {
            if let Some(command) = self.command_for(event.id.0.as_str()) {
                return Some(command);
            }
        }
        let icon = TrayIconEvent::receiver();
        while let Ok(event) = icon.try_recv() {
            if let TrayIconEvent::Click { button: tray_icon::MouseButton::Left, button_state: tray_icon::MouseButtonState::Up, .. } = event {
                return Some(TrayCommand::OpenSettings);
            }
        }
        None
    }

    fn command_for(&self, id: &str) -> Option<TrayCommand> {
        Some(match id {
            "toggle" => TrayCommand::SetEnabled(self.toggle.as_ref().is_none_or(CheckMenuItem::is_checked)),
            "restart" => TrayCommand::RetryHelper,
            "settings" => TrayCommand::OpenSettings,
            "quit" => TrayCommand::Quit,
            "room.join" => TrayCommand::RoomJoinClipboard,
            "room.create" => TrayCommand::RoomCreate,
            "room.copy" => TrayCommand::RoomCopyCode,
            "room.mute" => TrayCommand::RoomSetMuted(self.room_mute.as_ref().is_some_and(CheckMenuItem::is_checked)),
            "room.leave" => TrayCommand::RoomLeave,
            "room.settings" => TrayCommand::RoomOpenSettings,
            _ => return None,
        })
    }

    fn render(&mut self) {
        let t = self.text;
        let failed = self.mode == TrayMode::Failed;
        let icon = if failed { &self.icons.failed } else if self.mode == TrayMode::On { &self.icons.on } else { &self.icons.off };
        let _ = self.icon.set_icon(Some(icon.clone()));
        let tooltip = if failed {
            t.tray_tip_failed.to_string()
        } else {
            t.tray_tip(self.enabled)
        };
        let _ = self.icon.set_tooltip(Some(tooltip));

        let toggle = CheckMenuItem::with_id("toggle", t.tray_enabled, !failed, self.enabled, None);
        let mut items: Vec<Box<dyn tray_icon::menu::IsMenuItem>> = vec![Box::new(toggle.clone())];
        if failed {
            items.push(Box::new(MenuItem::with_id("restart", t.tray_restart, true, None)));
        }
        items.push(Box::new(PredefinedMenuItem::separator()));
        let room = self.room_items();
        let room_refs: Vec<&dyn tray_icon::menu::IsMenuItem> = room.iter().map(|item| item.as_ref()).collect();
        items.push(Box::new(Submenu::with_items(t.tray_room, true, &room_refs).expect("room menu")));
        items.push(Box::new(MenuItem::with_id("settings", t.tray_settings, true, None)));
        items.push(Box::new(MenuItem::with_id("quit", t.tray_quit, true, None)));

        let menu = Menu::new();
        let refs: Vec<&dyn tray_icon::menu::IsMenuItem> = items.iter().map(|item| item.as_ref()).collect();
        let _ = menu.append_items(&refs);
        self.icon.set_menu(Some(Box::new(menu)));
        self.toggle = Some(toggle);
    }

    fn room_items(&mut self) -> Vec<Box<dyn tray_icon::menu::IsMenuItem>> {
        let t = self.text;
        let settings = || Box::new(MenuItem::with_id("room.settings", t.tray_room_settings, true, None)) as Box<dyn tray_icon::menu::IsMenuItem>;
        let Some(code) = &self.room.code else {
            return vec![
                Box::new(MenuItem::with_id("room.join", t.tray_room_join_clipboard, true, None)),
                Box::new(MenuItem::with_id("room.create", t.tray_room_create, true, None)),
                settings(),
            ];
        };
        let mute = CheckMenuItem::with_id("room.mute", t.tray_room_mute, true, self.room.muted, None);
        self.room_mute = Some(mute.clone());
        vec![
            Box::new(MenuItem::new(t.tray_room_label(code, self.room.count), false, None)),
            Box::new(PredefinedMenuItem::separator()),
            Box::new(MenuItem::with_id("room.copy", t.tray_room_copy, true, None)),
            Box::new(mute),
            Box::new(MenuItem::with_id("room.leave", t.tray_room_leave, true, None)),
            Box::new(PredefinedMenuItem::separator()),
            settings(),
        ]
    }

    pub fn destroy(self) {
        let _ = self.icon.set_visible(false);
    }
}
