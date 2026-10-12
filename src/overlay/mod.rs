pub mod assets;
pub mod paint;
pub mod scene;
pub mod text;
pub mod window;

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tiny_skia::Pixmap;
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage};

use crate::display::{self, DisplayMap};
use crate::emotes::{self, EmoteArt};
use crate::geometry::{pick_slice, WHEEL};
use crate::pings;
use assets::{AnimPlayer, Assets};
use scene::{EmoteInstance, PingInstance, Scene, Tag, Toast, Visuals, Wheel, WheelItem, WheelKind};
use text::Text;
use window::LayeredWindow;

const FRAME: Duration = Duration::from_micros(16_667);
const PING_TAIL_S: f32 = 0.15;
const EMOTE_LIFE_S: f32 = 3.15;
const TOAST_LIFE_S: f32 = 1.8;
const TOAST_OUT_S: f32 = 0.22;
const WHEEL_CLOSE_S: f32 = 0.15;
const MAX_PINGS: usize = 60;
/// The owner key of this machine's own emote: one per screen, replaced by the next one.
const SELF: &str = "self";

#[derive(Clone, Debug)]
pub struct Paths {
    pub assets: PathBuf,
    pub bundled_emotes: PathBuf,
    pub custom_emotes: PathBuf,
    pub fonts: PathBuf,
}

pub enum Command {
    Visuals(Box<Visuals>),
    OpenWheel { display_id: u32, kind: WheelKind, x: i32, y: i32 },
    MoveWheel { x: i32, y: i32 },
    ReleaseWheel { x: i32, y: i32 },
    CancelWheel,
    SpawnPing { display_id: u32, ping: String, x: i32, y: i32, tag: Option<Tag> },
    SpawnEmote { display_id: u32, art: EmoteArt, owner: String, x: i32, y: i32, tag: Option<Tag> },
    ClearEmote { owner: String, except_display: Option<u32> },
    Toast { display_id: u32, title: String, body: String },
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum Event {
    Hover(Option<usize>),
    Released { kind: WheelKind, key: String, display_id: u32, x: f32, y: f32 },
    Cancelled,
}

pub struct Overlay {
    tx: Sender<Command>,
    join: Option<JoinHandle<()>>,
}

impl Overlay {
    pub fn start(paths: Paths, events: Sender<Event>) -> std::io::Result<Self> {
        let (tx, rx) = std::sync::mpsc::channel();
        let join = std::thread::Builder::new().name("overlay".into()).spawn(move || run(rx, events, paths))?;
        Ok(Self { tx, join: Some(join) })
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    pub fn shutdown(mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

struct Runtime {
    map: DisplayMap,
    window: LayeredWindow,
    scene: Scene,
    dirty: bool,
}

fn emote_icon(reference: &str, paths: &Paths) -> String {
    if emotes::is_custom_ref(reference) {
        paths.custom_emotes.join(format!("{}.webp", emotes::custom_hash(reference))).to_string_lossy().into_owned()
    } else {
        paths.bundled_emotes.join(format!("{reference}.png")).to_string_lossy().into_owned()
    }
}

fn wheel_hub(kind: WheelKind, visuals: &Visuals, paths: &Paths) -> String {
    match kind {
        WheelKind::Ping => pings::ping_by_id(&visuals.click_ping)
            .map(|p| paths.assets.join("textures").join(format!("{}.png", p.icon)).to_string_lossy().into_owned())
            .unwrap_or_default(),
        WheelKind::Emote => emote_icon(&visuals.click_emote, paths),
    }
}

fn wheel_items(kind: WheelKind, visuals: &Visuals, paths: &Paths) -> Vec<WheelItem> {
    match kind {
        WheelKind::Ping => visuals
            .wheel
            .iter()
            .filter_map(|id| pings::ping_by_id(id))
            .map(|p| WheelItem {
                key: p.id.to_string(),
                icon: paths.assets.join("textures").join(format!("{}.png", p.icon)).to_string_lossy().into_owned(),
            })
            .collect(),
        WheelKind::Emote => visuals
            .emote_wheel
            .iter()
            .map(|reference| WheelItem { key: reference.clone(), icon: emote_icon(reference, paths) })
            .collect(),
    }
}

fn run(rx: Receiver<Command>, events: Sender<Event>, paths: Paths) {
    let maps = display::displays();
    let mut runtimes: Vec<Runtime> = Vec::new();
    for map in &maps {
        match LayeredWindow::new(map.phys.x, map.phys.y, map.phys.width, map.phys.height) {
            Ok(window) => {
                window.show();
                runtimes.push(Runtime {
                    map: *map,
                    window,
                    scene: Scene { display_id: map.id, ..Default::default() },
                    dirty: true,
                });
            }
            Err(e) => eprintln!("overlay window for display {} failed: {e}", map.id),
        }
    }

    let mut assets = Assets::new();
    let text = Text::load(&paths.fonts);
    let mut visuals = Visuals::default();
    let mut msg = MSG::default();
    let mut was_active = false;
    let mut next_frame = Instant::now();
    // Survives the iteration boundary: a command handled while the loop waits must still repaint.
    let mut changed = false;

    loop {
        let mut shutdown = false;
        loop {
            match rx.try_recv() {
                Ok(Command::Shutdown) => {
                    shutdown = true;
                    break;
                }
                Ok(command) => {
                    changed |= handle(command, &maps, &mut runtimes, &mut visuals, &paths, &events);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    shutdown = true;
                    break;
                }
            }
        }
        if shutdown {
            break;
        }

        unsafe {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        let now = Instant::now();
        let mut active = false;
        for runtime in &mut runtimes {
            active |= advance(runtime, now);
        }
        if active || was_active || changed {
            for runtime in &mut runtimes {
                if active || was_active || runtime.dirty {
                    paint(runtime, &mut assets, &text, &visuals, now, &paths);
                }
            }
        }
        was_active = active;
        changed = false;

        if active {
            next_frame += FRAME;
            let now = Instant::now();
            if next_frame > now {
                std::thread::sleep(next_frame - now);
            } else {
                next_frame = now;
            }
        } else {
            next_frame = Instant::now();
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Command::Shutdown) => break,
                Ok(command) => {
                    changed |= handle(command, &maps, &mut runtimes, &mut visuals, &paths, &events);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    }
}

fn runtime_for(runtimes: &mut [Runtime], display_id: u32) -> Option<&mut Runtime> {
    runtimes.iter_mut().find(|r| r.map.id == display_id)
}

/// What a wheel release leaves on screen: the same ping or emote a click would send, at the wheel's centre.
fn spawn_at(
    runtimes: &mut [Runtime],
    display_id: u32,
    kind: WheelKind,
    key: &str,
    x: f32,
    y: f32,
    visuals: &Visuals,
    paths: &Paths,
) {
    match kind {
        WheelKind::Ping => {
            let Some(runtime) = runtime_for(runtimes, display_id) else {
                return;
            };
            while runtime.scene.pings.len() >= MAX_PINGS {
                runtime.scene.pings.remove(0);
            }
            runtime.scene.pings.push(PingInstance {
                ping: key.to_string(),
                x,
                y,
                start: Instant::now(),
                size_px: visuals.ping_size_px,
                duration_s: visuals.ping_duration_s,
                tag: None,
            });
            runtime.dirty = true;
        }
        WheelKind::Emote => {
            let art =
                emotes::resolve_emote_art(key, &visuals.custom_emotes, false, &paths.bundled_emotes, &paths.custom_emotes);
            for runtime in runtimes.iter_mut() {
                if runtime.map.id == display_id {
                    continue;
                }
                if runtime.scene.emotes.iter().any(|e| e.owner == SELF) {
                    runtime.scene.emotes.retain(|e| e.owner != SELF);
                    runtime.dirty = true;
                }
            }
            let Some(runtime) = runtime_for(runtimes, display_id) else {
                return;
            };
            runtime.scene.emotes.retain(|e| e.owner != SELF);
            runtime.scene.emotes.push(EmoteInstance {
                player: AnimPlayer::for_art(&art),
                art,
                owner: SELF.to_string(),
                x,
                y,
                start: Instant::now(),
                size_px: visuals.emote_size_px,
                tag: None,
            });
            runtime.dirty = true;
        }
    }
}

fn local_point(maps: &[DisplayMap], display_id: u32, x: i32, y: i32) -> Option<(f32, f32)> {
    let local = display::physical_to_local(x, y, maps, Some(display_id))?;
    Some((local.x as f32, local.y as f32))
}

fn handle(
    command: Command,
    maps: &[DisplayMap],
    runtimes: &mut [Runtime],
    visuals: &mut Visuals,
    paths: &Paths,
    events: &Sender<Event>,
) -> bool {
    match command {
        Command::Shutdown => {}
        Command::Visuals(next) => {
            *visuals = *next;
            for runtime in runtimes.iter_mut() {
                if let Some(wheel) = &mut runtime.scene.wheel {
                    wheel.items = wheel_items(wheel.kind, visuals, paths);
                    wheel.hub = wheel_hub(wheel.kind, visuals, paths);
                    runtime.dirty = true;
                }
            }
        }
        Command::OpenWheel { display_id, kind, x, y } => {
            let Some((lx, ly)) = local_point(maps, display_id, x, y) else {
                return false;
            };
            let items = wheel_items(kind, visuals, paths);
            let hub = wheel_hub(kind, visuals, paths);
            for runtime in runtimes.iter_mut() {
                if let Some(wheel) = &mut runtime.scene.wheel {
                    wheel.closed = Some(Instant::now());
                }
            }
            if let Some(runtime) = runtime_for(runtimes, display_id) {
                runtime.scene.wheel =
                    Some(Wheel { kind, x: lx, y: ly, hot: None, items, hub, opened: Instant::now(), closed: None });
                runtime.dirty = true;
            }
        }
        Command::MoveWheel { x, y } => {
            let Some(runtime) = runtimes.iter_mut().find(|r| r.scene.wheel.as_ref().is_some_and(|w| w.closed.is_none()))
            else {
                return false;
            };
            let Some(wheel) = &mut runtime.scene.wheel else {
                return false;
            };
            let Some((lx, ly)) = local_point(maps, runtime.map.id, x, y) else {
                return false;
            };
            let hot = pick_slice(lx - wheel.x, ly - wheel.y, WHEEL.inner_r);
            if hot != wheel.hot {
                wheel.hot = hot;
                runtime.dirty = true;
                let _ = events.send(Event::Hover(hot));
            }
        }
        Command::ReleaseWheel { x, y } => {
            let Some(runtime) = runtimes.iter_mut().find(|r| r.scene.wheel.as_ref().is_some_and(|w| w.closed.is_none()))
            else {
                return false;
            };
            let Some(wheel) = runtime.scene.wheel.take() else {
                return false;
            };
            runtime.dirty = true;
            let display_id = runtime.map.id;
            let slice = local_point(maps, display_id, x, y).and_then(|(lx, ly)| pick_slice(lx - wheel.x, ly - wheel.y, WHEEL.inner_r));
            let chosen = slice.and_then(|index| wheel.items.get(index)).cloned();
            match chosen {
                Some(item) => {
                    spawn_at(runtimes, display_id, wheel.kind, &item.key, wheel.x, wheel.y, visuals, paths);
                    let _ = events.send(Event::Released {
                        kind: wheel.kind,
                        key: item.key,
                        display_id,
                        x: wheel.x,
                        y: wheel.y,
                    });
                }
                None => {
                    let _ = events.send(Event::Cancelled);
                }
            }
        }
        Command::CancelWheel => {
            for runtime in runtimes.iter_mut() {
                if runtime.scene.wheel.take().is_some() {
                    runtime.dirty = true;
                    let _ = events.send(Event::Cancelled);
                }
            }
        }
        Command::SpawnPing { display_id, ping, x, y, tag } => {
            let Some((lx, ly)) = local_point(maps, display_id, x, y) else {
                return false;
            };
            if let Some(runtime) = runtime_for(runtimes, display_id) {
                while runtime.scene.pings.len() >= MAX_PINGS {
                    runtime.scene.pings.remove(0);
                }
                runtime.scene.pings.push(PingInstance {
                    ping,
                    x: lx,
                    y: ly,
                    start: Instant::now(),
                    size_px: visuals.ping_size_px,
                    duration_s: visuals.ping_duration_s,
                    tag,
                });
                runtime.dirty = true;
            }
        }
        Command::SpawnEmote { display_id, art, owner, x, y, tag } => {
            let Some((lx, ly)) = local_point(maps, display_id, x, y) else {
                return false;
            };
            for runtime in runtimes.iter_mut() {
                if runtime.map.id != display_id {
                    runtime.scene.emotes.retain(|e| e.owner != owner);
                    runtime.dirty = true;
                }
            }
            if let Some(runtime) = runtime_for(runtimes, display_id) {
                runtime.scene.emotes.retain(|e| e.owner != owner);
                runtime.scene.emotes.push(EmoteInstance {
                    player: AnimPlayer::for_art(&art),
                    art,
                    owner,
                    x: lx,
                    y: ly,
                    start: Instant::now(),
                    size_px: visuals.emote_size_px,
                    tag,
                });
                runtime.dirty = true;
            }
        }
        Command::ClearEmote { owner, except_display } => {
            for runtime in runtimes.iter_mut() {
                if Some(runtime.map.id) == except_display {
                    continue;
                }
                let before = runtime.scene.emotes.len();
                runtime.scene.emotes.retain(|e| e.owner != owner);
                if runtime.scene.emotes.len() != before {
                    runtime.dirty = true;
                }
            }
        }
        Command::Toast { display_id, title, body } => {
            if let Some(runtime) = runtime_for(runtimes, display_id) {
                runtime.scene.toast = Some(Toast { title, body, start: Instant::now(), leaving: None });
                runtime.dirty = true;
            }
        }
    }
    true
}

fn advance(runtime: &mut Runtime, now: Instant) -> bool {
    let scene = &mut runtime.scene;
    let before = scene.pings.len() + scene.emotes.len() + usize::from(scene.toast.is_some());
    scene.pings.retain(|p| now.duration_since(p.start).as_secs_f32() < p.duration_s + PING_TAIL_S);
    scene.emotes.retain(|e| now.duration_since(e.start).as_secs_f32() < EMOTE_LIFE_S);
    if let Some(toast) = &mut scene.toast {
        let age = now.duration_since(toast.start).as_secs_f32();
        match toast.leaving {
            Some(left) => {
                if now.duration_since(left).as_secs_f32() > TOAST_OUT_S {
                    scene.toast = None;
                }
            }
            None if age > TOAST_LIFE_S => toast.leaving = Some(now),
            None => {}
        }
    }
    if let Some(wheel) = &mut scene.wheel {
        if let Some(closed) = wheel.closed {
            if now.duration_since(closed).as_secs_f32() > WHEEL_CLOSE_S {
                scene.wheel = None;
            }
        }
    }
    let after = scene.pings.len() + scene.emotes.len() + usize::from(scene.toast.is_some());
    if before != after {
        runtime.dirty = true;
    }

    // Pings and emotes move for their whole life; the wheel and the toast only while they fade in or out.
    let toast_fading = scene.toast.as_ref().is_some_and(|toast| match toast.leaving {
        Some(left) => now.duration_since(left).as_secs_f32() < paint::TOAST_FADE_OUT_S,
        None => now.duration_since(toast.start).as_secs_f32() < paint::TOAST_FADE_IN_S,
    });
    let wheel_fading = scene.wheel.as_ref().is_some_and(|wheel| match wheel.closed {
        Some(_) => true,
        None => now.duration_since(wheel.opened).as_secs_f32() < paint::WHEEL_FADE_S,
    });
    scene.pings.len() + scene.emotes.len() > 0 || toast_fading || wheel_fading
}

fn paint(runtime: &mut Runtime, assets: &mut Assets, text: &Text, visuals: &Visuals, now: Instant, paths: &Paths) {
    let (width, height) = runtime.window.size();
    let Some(mut canvas) = Pixmap::new(width as u32, height as u32) else {
        return;
    };
    let mut ctx = paint::PaintCtx {
        canvas: &mut canvas,
        assets,
        text,
        visuals,
        now,
        scale: runtime.map.scale as f32,
        paths,
    };
    paint::paint(&mut runtime.scene, &mut ctx);
    if let Err(e) = runtime.window.present_rgba(canvas.data()) {
        eprintln!("overlay present failed: {e}");
    }
    runtime.dirty = false;
}
