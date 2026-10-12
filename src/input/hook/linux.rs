use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::NONE;

use crate::input::decision::{
    Btn, Config, Decision, Emit, EvKind, Inject, InjectKind, InputEvent, Outcome, Trigger, MOD_ALT, MOD_CTRL, MOD_SHIFT,
    MOD_WIN,
};

const MASTER_DEVICES: u16 = 1;
const POLL: Duration = Duration::from_millis(4);
const WATCHDOG: Duration = Duration::from_millis(1000);

const XK_BACK_SPACE: u32 = 0xff08;
const XK_TAB: u32 = 0xff09;
const XK_RETURN: u32 = 0xff0d;
const XK_ESCAPE: u32 = 0xff1b;
const XK_HOME: u32 = 0xff50;
const XK_LEFT: u32 = 0xff51;
const XK_UP: u32 = 0xff52;
const XK_RIGHT: u32 = 0xff53;
const XK_DOWN: u32 = 0xff54;
const XK_PAGE_UP: u32 = 0xff55;
const XK_PAGE_DOWN: u32 = 0xff56;
const XK_END: u32 = 0xff57;
const XK_INSERT: u32 = 0xff63;
const XK_PAUSE: u32 = 0xff13;
const XK_SCROLL_LOCK: u32 = 0xff14;
const XK_PRINT: u32 = 0xff61;
const XK_NUM_LOCK: u32 = 0xff7f;
const XK_F1: u32 = 0xffbe;
const XK_SHIFT_L: u32 = 0xffe1;
const XK_SHIFT_R: u32 = 0xffe2;
const XK_CONTROL_L: u32 = 0xffe3;
const XK_CONTROL_R: u32 = 0xffe4;
const XK_CAPS_LOCK: u32 = 0xffe5;
const XK_ALT_L: u32 = 0xffe9;
const XK_ALT_R: u32 = 0xffea;
const XK_SUPER_L: u32 = 0xffeb;
const XK_SUPER_R: u32 = 0xffec;
const XK_DELETE: u32 = 0xffff;

const VK_BACK: u32 = 0x08;
const VK_TAB: u32 = 0x09;
const VK_RETURN: u32 = 0x0D;
const VK_PAUSE: u32 = 0x13;
const VK_CAPITAL: u32 = 0x14;
const VK_ESCAPE: u32 = 0x1B;
const VK_SPACE: u32 = 0x20;
const VK_PRIOR: u32 = 0x21;
const VK_NEXT: u32 = 0x22;
const VK_END: u32 = 0x23;
const VK_HOME: u32 = 0x24;
const VK_LEFT: u32 = 0x25;
const VK_UP: u32 = 0x26;
const VK_RIGHT: u32 = 0x27;
const VK_DOWN: u32 = 0x28;
const VK_SNAPSHOT: u32 = 0x2C;
const VK_INSERT: u32 = 0x2D;
const VK_DELETE: u32 = 0x2E;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_F1: u32 = 0x70;
const VK_NUMLOCK: u32 = 0x90;
const VK_SCROLL: u32 = 0x91;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;
const VK_OEM_1: u32 = 0xBA;
const VK_OEM_PLUS: u32 = 0xBB;
const VK_OEM_COMMA: u32 = 0xBC;
const VK_OEM_MINUS: u32 = 0xBD;
const VK_OEM_PERIOD: u32 = 0xBE;
const VK_OEM_2: u32 = 0xBF;
const VK_OEM_3: u32 = 0xC0;
const VK_OEM_4: u32 = 0xDB;
const VK_OEM_5: u32 = 0xDC;
const VK_OEM_6: u32 = 0xDD;
const VK_OEM_7: u32 = 0xDE;

pub enum HookCommand {
    Config(Config),
    Enabled(bool),
    Suspend(bool),
    Shutdown,
}

pub struct HookHandle {
    tx: Sender<HookCommand>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl HookHandle {
    pub fn send(&self, cmd: HookCommand) -> bool {
        self.tx.send(cmd).is_ok()
    }

    pub fn shutdown(mut self) {
        let _ = self.tx.send(HookCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Grabs the trigger combinations on a thread of its own and reports once they are in place.
pub fn spawn(sink: Sender<Emit>, ready: Sender<Result<u32, String>>) -> io::Result<HookHandle> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (up_tx, up_rx) = std::sync::mpsc::channel();
    let join = std::thread::Builder::new().name("input-hook".into()).spawn(move || run(sink, ready, rx, up_tx))?;
    let _ = up_rx.recv();
    Ok(HookHandle { tx, join: Some(join) })
}

struct Keymap {
    vk: [u32; 256],
    keycode: HashMap<u32, xproto::Keycode>,
}

impl Keymap {
    fn is_down(&self, keys: &[u8; 32], vk: u32) -> bool {
        let Some(&keycode) = self.keycode.get(&vk) else {
            return false;
        };
        keys[(keycode / 8) as usize] & (1 << (keycode % 8)) != 0
    }
}

struct State {
    conn: RustConnection,
    root: xproto::Window,
    keymap: Keymap,
    decision: Decision,
    sink: Sender<Emit>,
    cfg: Config,
    buttons: Vec<(xproto::ButtonIndex, xproto::ModMask)>,
    keys: Vec<(xproto::Keycode, xproto::ModMask)>,
    fullscreen_atom: xproto::Atom,
    wm_state_atom: xproto::Atom,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|cell| {
        let mut guard = cell.borrow_mut();
        let st = guard.as_mut()?;
        Some(f(st))
    })
}

fn now_ms() -> u64 {
    static START: std::sync::LazyLock<Instant> = std::sync::LazyLock::new(Instant::now);
    START.elapsed().as_millis() as u64
}

fn keysym_to_vk(keysym: u32) -> Option<u32> {
    Some(match keysym {
        XK_BACK_SPACE => VK_BACK,
        XK_TAB => VK_TAB,
        XK_RETURN => VK_RETURN,
        XK_ESCAPE => VK_ESCAPE,
        XK_HOME => VK_HOME,
        XK_LEFT => VK_LEFT,
        XK_UP => VK_UP,
        XK_RIGHT => VK_RIGHT,
        XK_DOWN => VK_DOWN,
        XK_PAGE_UP => VK_PRIOR,
        XK_PAGE_DOWN => VK_NEXT,
        XK_END => VK_END,
        XK_INSERT => VK_INSERT,
        XK_DELETE => VK_DELETE,
        XK_PAUSE => VK_PAUSE,
        XK_SCROLL_LOCK => VK_SCROLL,
        XK_PRINT => VK_SNAPSHOT,
        XK_NUM_LOCK => VK_NUMLOCK,
        XK_SHIFT_L => VK_LSHIFT,
        XK_SHIFT_R => VK_RSHIFT,
        XK_CONTROL_L => VK_LCONTROL,
        XK_CONTROL_R => VK_RCONTROL,
        XK_ALT_L => VK_LMENU,
        XK_ALT_R => VK_RMENU,
        XK_SUPER_L => VK_LWIN,
        XK_SUPER_R => VK_RWIN,
        XK_CAPS_LOCK => VK_CAPITAL,
        0x0020 => VK_SPACE,
        0x0030..=0x0039 => keysym - 0x0030 + 0x30,
        0x0061..=0x007a => keysym - 0x0061 + 0x41,
        0x0027 => VK_OEM_7,
        0x002c => VK_OEM_COMMA,
        0x002d => VK_OEM_MINUS,
        0x002e => VK_OEM_PERIOD,
        0x002f => VK_OEM_2,
        0x003b => VK_OEM_1,
        0x003d => VK_OEM_PLUS,
        0x005b => VK_OEM_4,
        0x005c => VK_OEM_5,
        0x005d => VK_OEM_6,
        0x0060 => VK_OEM_3,
        0xffbe..=0xffd5 => keysym - XK_F1 + VK_F1,
        _ => return None,
    })
}

fn build_keymap(conn: &RustConnection, setup: &xproto::Setup) -> Result<Keymap, String> {
    let min = setup.min_keycode;
    let max = setup.max_keycode;
    let reply = conn
        .get_keyboard_mapping(min, max - min + 1)
        .map_err(|e| e.to_string())?
        .reply()
        .map_err(|e| e.to_string())?;
    let per = usize::from(reply.keysyms_per_keycode);
    let mut vk = [0u32; 256];
    let mut keycode = HashMap::new();
    for code in min..=max {
        let base = usize::from(code - min) * per;
        for slot in 0..per {
            let keysym = reply.keysyms.get(base + slot).copied().unwrap_or(0);
            if let Some(mapped) = keysym_to_vk(keysym) {
                vk[usize::from(code)] = mapped;
                keycode.entry(mapped).or_insert(code);
                break;
            }
        }
    }
    Ok(Keymap { vk, keycode })
}

fn modifier_mask(trigger: Trigger) -> Option<xproto::ModMask> {
    Some(match trigger {
        Trigger::Alt => xproto::ModMask::M1,
        Trigger::Ctrl => xproto::ModMask::CONTROL,
        Trigger::Shift => xproto::ModMask::SHIFT,
        Trigger::Win => xproto::ModMask::M4,
        _ => return None,
    })
}

fn decision_mods(mods: u32) -> xproto::ModMask {
    let mut mask = xproto::ModMask::default();
    if mods & MOD_SHIFT != 0 {
        mask |= xproto::ModMask::SHIFT;
    }
    if mods & MOD_CTRL != 0 {
        mask |= xproto::ModMask::CONTROL;
    }
    if mods & MOD_ALT != 0 {
        mask |= xproto::ModMask::M1;
    }
    if mods & MOD_WIN != 0 {
        mask |= xproto::ModMask::M4;
    }
    mask
}

fn lock_variants(mask: xproto::ModMask) -> [xproto::ModMask; 4] {
    let locks = xproto::ModMask::LOCK | xproto::ModMask::M2;
    [mask, mask | xproto::ModMask::LOCK, mask | xproto::ModMask::M2, mask | locks]
}

fn trigger_keycode(st: &State, trigger: Trigger, vk: u32) -> Option<xproto::Keycode> {
    let wanted = if trigger == Trigger::CapsLock { VK_CAPITAL } else { vk };
    st.keymap.keycode.get(&wanted).copied()
}

fn apply_grabs(st: &mut State) {
    for (button, mask) in st.buttons.clone() {
        let _ = st.conn.ungrab_button(button, st.root, mask);
    }
    for (keycode, mask) in st.keys.clone() {
        let _ = st.conn.ungrab_key(keycode, st.root, mask);
    }
    st.buttons.clear();
    st.keys.clear();

    let mask = xproto::EventMask::BUTTON_PRESS | xproto::EventMask::BUTTON_RELEASE | xproto::EventMask::POINTER_MOTION;
    for spec in [st.cfg.trigger, st.cfg.emote_trigger] {
        if let Some(trigger_mask) = modifier_mask(spec) {
            for variant in lock_variants(trigger_mask) {
                let _ = st.conn.grab_button(
                    false,
                    st.root,
                    mask,
                    xproto::GrabMode::ASYNC,
                    xproto::GrabMode::ASYNC,
                    NONE,
                    NONE,
                    xproto::ButtonIndex::M1,
                    variant,
                );
                st.buttons.push((xproto::ButtonIndex::M1, variant));
            }
        } else if spec == Trigger::Mouse4 || spec == Trigger::Mouse5 {
            let button = xproto::ButtonIndex::from(if spec == Trigger::Mouse4 { 8u8 } else { 9u8 });
            let _ = st.conn.grab_button(
                false,
                st.root,
                mask,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
                NONE,
                NONE,
                button,
                xproto::ModMask::ANY,
            );
            st.buttons.push((button, xproto::ModMask::ANY));
        } else if spec == Trigger::CapsLock || spec == Trigger::CustomVk {
            let vk = if spec == Trigger::CapsLock { 0 } else { st.cfg.trigger_vk };
            if let Some(keycode) = trigger_keycode(st, spec, vk) {
                for variant in lock_variants(xproto::ModMask::default()) {
                    let _ = st.conn.grab_key(
                        false,
                        st.root,
                        variant,
                        keycode,
                        xproto::GrabMode::ASYNC,
                        xproto::GrabMode::ASYNC,
                    );
                    st.keys.push((keycode, variant));
                }
            }
        }
    }

    if st.cfg.toggle_vk != 0 {
        if let Some(keycode) = st.keymap.keycode.get(&(st.cfg.toggle_vk & 0xFF)).copied() {
            for variant in lock_variants(decision_mods(st.cfg.toggle_mods)) {
                let _ = st.conn.grab_key(
                    false,
                    st.root,
                    variant,
                    keycode,
                    xproto::GrabMode::ASYNC,
                    xproto::GrabMode::ASYNC,
                );
                st.keys.push((keycode, variant));
            }
        }
    }
    let _ = st.conn.flush();
}

fn button_code(btn: Btn) -> xproto::ButtonIndex {
    xproto::ButtonIndex::from(match btn {
        Btn::Left => 1u8,
        Btn::Middle => 2u8,
        Btn::Right => 3u8,
        Btn::X1 => 8u8,
        Btn::X2 => 9u8,
        Btn::None => 0u8,
    })
}

fn button_kind(button: xproto::ButtonIndex) -> Btn {
    match u8::from(button) {
        1 => Btn::Left,
        2 => Btn::Middle,
        3 => Btn::Right,
        8 => Btn::X1,
        9 => Btn::X2,
        _ => Btn::None,
    }
}

fn inject_all(st: &mut State, injects: &[Inject]) {
    let buttons = injects.iter().any(|i| matches!(i.kind, InjectKind::ButtonDown | InjectKind::ButtonUp));
    // Replaying a click would land back in our own grab, so the grab steps aside for the injection.
    if buttons {
        for (button, mask) in st.buttons.clone() {
            let _ = st.conn.ungrab_button(button, st.root, mask);
        }
    }
    for inject in injects {
        let (type_, detail) = match inject.kind {
            InjectKind::ButtonDown => (xproto::BUTTON_PRESS_EVENT, u8::from(button_code(inject.btn))),
            InjectKind::ButtonUp => (xproto::BUTTON_RELEASE_EVENT, u8::from(button_code(inject.btn))),
            InjectKind::KeyDown => {
                let Some(&keycode) = st.keymap.keycode.get(&inject.vk) else {
                    continue;
                };
                (xproto::KEY_PRESS_EVENT, keycode)
            }
            InjectKind::KeyUp => {
                let Some(&keycode) = st.keymap.keycode.get(&inject.vk) else {
                    continue;
                };
                (xproto::KEY_RELEASE_EVENT, keycode)
            }
        };
        let _ = st.conn.xtest_fake_input(type_, detail, x11rb::CURRENT_TIME, st.root, 0, 0, 0);
    }
    if buttons {
        for (button, mask) in st.buttons.clone() {
            let _ = st.conn.grab_button(
                false,
                st.root,
                xproto::EventMask::BUTTON_PRESS | xproto::EventMask::BUTTON_RELEASE | xproto::EventMask::POINTER_MOTION,
                xproto::GrabMode::ASYNC,
                xproto::GrabMode::ASYNC,
                NONE,
                NONE,
                button,
                mask,
            );
        }
    }
    let _ = st.conn.flush();
}

fn apply(outcome: Outcome) {
    if outcome.emits.is_empty() && outcome.injects.is_empty() {
        return;
    }
    with_state(|st| {
        for emit in &outcome.emits {
            let _ = st.sink.send(*emit);
        }
        if !outcome.injects.is_empty() {
            inject_all(st, &outcome.injects);
        }
    });
}

fn sync_keys(st: &mut State) {
    let Ok(cookie) = st.conn.query_keymap() else {
        return;
    };
    let Ok(reply) = cookie.reply() else {
        return;
    };
    let keymap = &st.keymap;
    let keys = &reply.keys;
    st.decision.sync_key_states(|vk| keymap.is_down(keys, vk));
}

fn key_event(keycode: u32, down: bool) {
    let outcome = with_state(|st| {
        let vk = st.keymap.vk[usize::from((keycode & 0xFF) as u8)];
        if vk == 0 {
            return None;
        }
        let event = InputEvent {
            kind: if down { EvKind::KeyDown } else { EvKind::KeyUp },
            vk,
            time_ms: now_ms(),
            ..Default::default()
        };
        Some(st.decision.on_event(&event))
    });
    if let Some(Some(outcome)) = outcome {
        apply(outcome);
    }
}

fn mouse_event(kind: EvKind, btn: Btn, x: i16, y: i16) {
    let outcome = with_state(|st| {
        if kind == EvKind::MouseDown {
            sync_keys(st);
        }
        let event = InputEvent { kind, btn, x: i32::from(x), y: i32::from(y), time_ms: now_ms(), ..Default::default() };
        st.decision.on_event(&event)
    });
    if let Some(outcome) = outcome {
        apply(outcome);
    }
}

/// True when the window under (x, y) covers its monitor as a fullscreen window.
fn click_is_on_fullscreen_app(x: i32, y: i32) -> bool {
    with_state(|st| {
        let _ = x;
        let _ = y;
        let Ok(cookie) = st.conn.query_pointer(st.root) else {
            return false;
        };
        let Ok(reply) = cookie.reply() else {
            return false;
        };
        if reply.child == NONE {
            return false;
        }
        let Ok(cookie) =
            st.conn.get_property(false, reply.child, st.wm_state_atom, u32::from(u16::from(xproto::AtomEnum::ATOM)), 0, 8)
        else {
            return false;
        };
        let Ok(state) = cookie.reply() else {
            return false;
        };
        state.value32().is_some_and(|mut atoms| atoms.any(|a| a == st.fullscreen_atom))
    })
    .unwrap_or(false)
}

fn handle_event(event: Event) {
    match event {
        Event::XinputRawKeyPress(e) => key_event(e.detail, true),
        Event::XinputRawKeyRelease(e) => key_event(e.detail, false),
        Event::ButtonPress(e) => mouse_event(
            EvKind::MouseDown,
            button_kind(xproto::ButtonIndex::from(e.detail)),
            e.root_x,
            e.root_y,
        ),
        Event::ButtonRelease(e) => mouse_event(
            EvKind::MouseUp,
            button_kind(xproto::ButtonIndex::from(e.detail)),
            e.root_x,
            e.root_y,
        ),
        Event::MotionNotify(e) => mouse_event(EvKind::MouseMove, Btn::None, e.root_x, e.root_y),
        Event::MappingNotify(_) => {
            with_state(|st| {
                if let Ok(keymap) = build_keymap(&st.conn, st.conn.setup()) {
                    st.keymap = keymap;
                }
                apply_grabs(st);
            });
        }
        _ => {}
    }
}

fn on_command(cmd: HookCommand) {
    let outcome = with_state(|st| match cmd {
        HookCommand::Config(cfg) => {
            st.cfg = cfg;
            apply_grabs(st);
            st.decision.set_config(cfg)
        }
        HookCommand::Enabled(on) => st.decision.set_enabled(on),
        HookCommand::Suspend(on) => st.decision.set_suspended(on),
        HookCommand::Shutdown => Outcome::default(),
    });
    if let Some(outcome) = outcome {
        apply(outcome);
    }
}

fn on_tick() {
    let outcome = with_state(|st| {
        sync_keys(st);
        st.decision.tick(now_ms())
    });
    if let Some(outcome) = outcome {
        apply(outcome);
    }
}

fn intern_atom(conn: &RustConnection, name: &[u8]) -> xproto::Atom {
    conn.intern_atom(false, name).ok().and_then(|c| c.reply().ok()).map(|r| r.atom).unwrap_or(NONE)
}

fn run(sink: Sender<Emit>, ready: Sender<Result<u32, String>>, rx: Receiver<HookCommand>, up_tx: Sender<()>) {
    let (conn, screen) = match x11rb::connect(None) {
        Ok(pair) => pair,
        Err(err) => {
            let _ = up_tx.send(());
            let _ = ready.send(Err(format!("X connection: {err}")));
            return;
        }
    };
    let root = conn.setup().roots[screen].root;
    let keymap = match build_keymap(&conn, conn.setup()) {
        Ok(keymap) => keymap,
        Err(err) => {
            let _ = up_tx.send(());
            let _ = ready.send(Err(err));
            return;
        }
    };

    let mut decision = Decision::new();
    // X11 has no Alt/Win menu activation to mask, and faking a Super press opens the desktop's overview.
    decision.set_mask_menu_keys(false);
    decision.set_fullscreen_probe(click_is_on_fullscreen_app);

    let event_mask = xinput::EventMask {
        deviceid: MASTER_DEVICES,
        mask: vec![xinput::XIEventMask::RAW_KEY_PRESS, xinput::XIEventMask::RAW_KEY_RELEASE],
    };
    if conn.xinput_xi_select_events(root, &[event_mask]).is_err() {
        let _ = up_tx.send(());
        let _ = ready.send(Err("XInput2 raw key events".into()));
        return;
    }

    STATE.with(|cell| {
        *cell.borrow_mut() = Some(State {
            conn,
            root,
            keymap,
            decision,
            sink,
            cfg: Config::default(),
            buttons: Vec::new(),
            keys: Vec::new(),
            fullscreen_atom: NONE,
            wm_state_atom: NONE,
        });
    });
    with_state(|st| {
        st.fullscreen_atom = intern_atom(&st.conn, b"_NET_WM_STATE_FULLSCREEN");
        st.wm_state_atom = intern_atom(&st.conn, b"_NET_WM_STATE");
        apply_grabs(st);
    });

    let _ = up_tx.send(());
    let _ = ready.send(Ok(0));

    let mut last_tick = Instant::now();
    let mut running = true;
    while running {
        let mut quiet = true;
        loop {
            let event = with_state(|st| st.conn.poll_for_event().ok().flatten());
            match event {
                Some(Some(event)) => {
                    quiet = false;
                    handle_event(event);
                }
                _ => break,
            }
        }
        loop {
            match rx.try_recv() {
                Ok(HookCommand::Shutdown) => {
                    running = false;
                    break;
                }
                Ok(cmd) => on_command(cmd),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    running = false;
                    break;
                }
            }
        }
        if last_tick.elapsed() >= WATCHDOG {
            last_tick = Instant::now();
            on_tick();
        }
        if quiet {
            std::thread::sleep(POLL);
        }
    }

    with_state(|st| {
        for (button, mask) in st.buttons.clone() {
            let _ = st.conn.ungrab_button(button, st.root, mask);
        }
        for (keycode, mask) in st.keys.clone() {
            let _ = st.conn.ungrab_key(keycode, st.root, mask);
        }
        let _ = st.conn.flush();
    });
    STATE.with(|cell| {
        *cell.borrow_mut() = None;
    });
}
