use std::collections::BTreeSet;

pub const MOD_CTRL: u32 = 1;
pub const MOD_ALT: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;

/// VK_NONAME: a harmless key that stops Alt/Win menu activation.
pub const MASK_VK: u32 = 0xFC;

pub const WATCHDOG_MS: u64 = 10_000;

const VK_SHIFT: u32 = 0x10;
const VK_CTRL: u32 = 0x11;
const VK_ALT: u32 = 0x12;
const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;
const VK_CAPITAL: u32 = 0x14;
const VK_ESCAPE: u32 = 0x1B;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Btn {
    #[default]
    None,
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum EvKind {
    MouseDown,
    MouseUp,
    #[default]
    MouseMove,
    KeyDown,
    KeyUp,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct InputEvent {
    pub kind: EvKind,
    pub btn: Btn,
    pub vk: u32,
    pub x: i32,
    pub y: i32,
    /// Injected by this app (marker in dwExtraInfo).
    pub self_injected: bool,
    pub time_ms: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Trigger {
    #[default]
    Alt,
    Ctrl,
    Shift,
    Win,
    CapsLock,
    Mouse4,
    Mouse5,
    CustomVk,
    /// No trigger: the emote key is off.
    None,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WheelKind {
    #[default]
    Ping,
    Emote,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Config {
    pub trigger: Trigger,
    pub trigger_vk: u32,
    pub click_ping: bool,
    pub game_check: bool,
    pub emote_trigger: Trigger,
    pub emote_trigger_vk: u32,
    pub emote_click: bool,
    pub drag_threshold_px: i32,
    pub toggle_mods: u32,
    pub toggle_vk: u32,
    pub enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            trigger: Trigger::Alt,
            trigger_vk: 0,
            click_ping: false,
            game_check: false,
            emote_trigger: Trigger::None,
            emote_trigger_vk: 0,
            emote_click: false,
            drag_threshold_px: 8,
            toggle_mods: MOD_CTRL | MOD_ALT,
            toggle_vk: 0x50,
            enabled: true,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EmitKind {
    WheelOpen,
    WheelMove,
    WheelRelease,
    Click,
    Cancel,
    Toggled,
}

#[derive(Clone, Copy, Debug)]
pub struct Emit {
    pub kind: EmitKind,
    pub x: i32,
    pub y: i32,
    /// Point events only: the wheel whose trigger started the gesture.
    pub wheel: WheelKind,
    /// Toggled only.
    pub enabled: bool,
}

impl Emit {
    fn plain(kind: EmitKind) -> Self {
        Self { kind, x: 0, y: 0, wheel: WheelKind::Ping, enabled: false }
    }

    fn point(kind: EmitKind, x: i32, y: i32, wheel: WheelKind) -> Self {
        Self { kind, x, y, wheel, enabled: false }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InjectKind {
    ButtonDown,
    ButtonUp,
    KeyDown,
    KeyUp,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Inject {
    pub kind: InjectKind,
    pub btn: Btn,
    pub vk: u32,
}

impl Inject {
    fn button(kind: InjectKind, btn: Btn) -> Self {
        Self { kind, btn, vk: 0 }
    }

    fn key(kind: InjectKind, vk: u32) -> Self {
        Self { kind, btn: Btn::None, vk }
    }
}

#[derive(Default, Debug)]
pub struct Outcome {
    pub swallow: bool,
    pub emits: Vec<Emit>,
    pub injects: Vec<Inject>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum State {
    #[default]
    Idle,
    Pending,
    Wheel,
    SwallowUp,
}

fn mod_bit(vk: u32) -> u32 {
    match vk {
        VK_CTRL | VK_LCONTROL | VK_RCONTROL => MOD_CTRL,
        VK_ALT | VK_LMENU | VK_RMENU => MOD_ALT,
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => MOD_SHIFT,
        VK_LWIN | VK_RWIN => MOD_WIN,
        _ => 0,
    }
}

/// One wheel's trigger settings, read from the ping or emote fields of the config.
#[derive(Clone, Copy)]
struct Spec {
    trigger: Trigger,
    vk: u32,
    click: bool,
}

/// Decides what happens to every input event. Pure logic: no Windows calls.
pub struct Decision {
    cfg: Config,
    enabled: bool,
    suspended: bool,
    fullscreen_probe: Option<fn(i32, i32) -> bool>,
    state: State,
    key_down: [bool; 256],
    swallow_key_ups: BTreeSet<u32>,
    swallow_right_up: bool,
    need_mask: bool,
    foreign_key: bool,
    mask_menu_keys: bool,
    owner: WheelKind,
    drag_btn: Btn,
    px: i32,
    py: i32,
    last_mouse_ms: u64,
}

impl Default for Decision {
    fn default() -> Self {
        Self {
            cfg: Config::default(),
            enabled: true,
            suspended: false,
            fullscreen_probe: None,
            state: State::Idle,
            key_down: [false; 256],
            swallow_key_ups: BTreeSet::new(),
            swallow_right_up: false,
            need_mask: false,
            foreign_key: false,
            mask_menu_keys: true,
            owner: WheelKind::Ping,
            drag_btn: Btn::None,
            px: 0,
            py: 0,
            last_mouse_ms: 0,
        }
    }
}

impl Decision {
    pub fn new() -> Self {
        Self::default()
    }

    /// macOS has no Alt/Win menu activation, so its hook layer turns this off.
    pub fn set_mask_menu_keys(&mut self, on: bool) {
        self.mask_menu_keys = on;
    }

    pub fn set_fullscreen_probe(&mut self, probe: fn(i32, i32) -> bool) {
        self.fullscreen_probe = Some(probe);
    }

    pub fn state(&self) -> State {
        self.state
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn set_config(&mut self, c: Config) -> Outcome {
        let mut r = Outcome::default();
        self.cancel_gesture(&mut r);
        self.cfg = c;
        self.enabled = c.enabled;
        r
    }

    pub fn set_enabled(&mut self, on: bool) -> Outcome {
        let mut r = Outcome::default();
        if !on {
            self.cancel_gesture(&mut r);
        }
        self.enabled = on;
        r
    }

    pub fn set_suspended(&mut self, on: bool) -> Outcome {
        let mut r = Outcome::default();
        if on {
            self.cancel_gesture(&mut r);
        }
        self.suspended = on;
        r
    }

    pub fn tick(&mut self, now_ms: u64) -> Outcome {
        let mut r = Outcome::default();
        let busy = self.state == State::Pending || self.state == State::Wheel;
        if busy && now_ms > self.last_mouse_ms && now_ms - self.last_mouse_ms > WATCHDOG_MS {
            r.emits.push(Emit::plain(EmitKind::Cancel));
            // The press was swallowed, so its up must be too; a later drag-button down resets SwallowUp.
            self.state = State::SwallowUp;
        }
        r
    }

    /// Resync with the OS: forget keys we believe are down but it reports up, never the reverse.
    pub fn sync_key_states(&mut self, is_down: impl Fn(u32) -> bool) {
        for vk in 0..256u32 {
            let swallowed = self.swallow_key_ups.contains(&vk);
            if self.key_down[vk as usize] && !swallowed && !is_down(vk) {
                self.key_down[vk as usize] = false;
            }
        }
        if !self.held(WheelKind::Ping) && !self.held(WheelKind::Emote) {
            self.foreign_key = false;
        }
    }

    pub fn on_event(&mut self, e: &InputEvent) -> Outcome {
        let mut r = Outcome::default();
        if e.self_injected {
            return r;
        }
        match e.kind {
            EvKind::KeyDown => self.on_key_down(e, &mut r),
            EvKind::KeyUp => self.on_key_up(e, &mut r),
            EvKind::MouseDown => {
                self.last_mouse_ms = e.time_ms;
                self.on_mouse_down(e, &mut r);
            }
            EvKind::MouseUp => {
                self.last_mouse_ms = e.time_ms;
                self.on_mouse_up(e, &mut r);
            }
            EvKind::MouseMove => {
                self.last_mouse_ms = e.time_ms;
                self.on_mouse_move(e, &mut r);
            }
        }
        r
    }

    fn active(&self) -> bool {
        self.enabled && !self.suspended
    }

    fn mods(&self) -> u32 {
        let mut m = 0;
        for vk in 0..256u32 {
            if self.key_down[vk as usize] {
                m |= mod_bit(vk);
            }
        }
        m
    }

    fn spec(&self, k: WheelKind) -> Spec {
        match k {
            WheelKind::Emote => Spec { trigger: self.cfg.emote_trigger, vk: self.cfg.emote_trigger_vk, click: self.cfg.emote_click },
            WheelKind::Ping => Spec { trigger: self.cfg.trigger, vk: self.cfg.trigger_vk, click: self.cfg.click_ping },
        }
    }

    /// Keyboard triggers only: a mouse trigger, or none, is never "held".
    fn held(&self, k: WheelKind) -> bool {
        match self.spec(k).trigger {
            Trigger::Alt => self.mods() & MOD_ALT != 0,
            Trigger::Ctrl => self.mods() & MOD_CTRL != 0,
            Trigger::Shift => self.mods() & MOD_SHIFT != 0,
            Trigger::Win => self.mods() & MOD_WIN != 0,
            Trigger::CapsLock | Trigger::CustomVk => self.key_down[(self.key_trigger_vk(k) & 0xFF) as usize],
            Trigger::Mouse4 | Trigger::Mouse5 | Trigger::None => false,
        }
    }

    fn is_trigger_vk(&self, k: WheelKind, vk: u32) -> bool {
        match self.spec(k).trigger {
            Trigger::Alt => mod_bit(vk) == MOD_ALT,
            Trigger::Ctrl => mod_bit(vk) == MOD_CTRL,
            Trigger::Shift => mod_bit(vk) == MOD_SHIFT,
            Trigger::Win => mod_bit(vk) == MOD_WIN,
            Trigger::CapsLock | Trigger::CustomVk => vk == self.key_trigger_vk(k),
            Trigger::Mouse4 | Trigger::Mouse5 | Trigger::None => false,
        }
    }

    fn is_mouse_trigger(&self, k: WheelKind) -> bool {
        matches!(self.spec(k).trigger, Trigger::Mouse4 | Trigger::Mouse5)
    }

    fn is_key_trigger(&self, k: WheelKind) -> bool {
        matches!(self.spec(k).trigger, Trigger::CapsLock | Trigger::CustomVk)
    }

    fn key_trigger_vk(&self, k: WheelKind) -> u32 {
        let s = self.spec(k);
        if s.trigger == Trigger::CapsLock {
            VK_CAPITAL
        } else {
            s.vk
        }
    }

    fn drag_button(&self, k: WheelKind) -> Btn {
        match self.spec(k).trigger {
            Trigger::Mouse4 => Btn::X1,
            Trigger::Mouse5 => Btn::X2,
            _ => Btn::Left,
        }
    }

    /// A press starts a gesture for this wheel: its side button, or the left button while its key is held.
    fn starts_gesture(&self, k: WheelKind, btn: Btn) -> bool {
        btn == self.drag_button(k) && (self.is_mouse_trigger(k) || self.held(k))
    }

    fn click_on(&self, k: WheelKind) -> bool {
        self.spec(k).click
    }

    /// Something was swallowed while Alt or Win is held: their key-up must be masked.
    fn mark_swallow(&mut self) {
        if self.mask_menu_keys && self.mods() & (MOD_ALT | MOD_WIN) != 0 {
            self.need_mask = true;
        }
    }

    fn cancel_gesture(&mut self, r: &mut Outcome) {
        if self.state == State::Pending || self.state == State::Wheel {
            r.emits.push(Emit::plain(EmitKind::Cancel));
            self.state = State::SwallowUp;
        }
    }

    fn on_key_down(&mut self, e: &InputEvent, r: &mut Outcome) {
        let vk = e.vk & 0xFF;
        let repeat = self.key_down[vk as usize];
        self.key_down[vk as usize] = true;

        // Another program's chord (Alt+Tab, Alt+F4…): until every trigger is released, clicks are its own.
        if self.state == State::Idle
            && mod_bit(vk) == 0
            && !self.is_trigger_vk(WheelKind::Ping, vk)
            && !self.is_trigger_vk(WheelKind::Emote, vk)
            && (self.held(WheelKind::Ping) || self.held(WheelKind::Emote))
        {
            self.foreign_key = true;
        }

        if !self.suspended && vk == self.cfg.toggle_vk && self.mods() == self.cfg.toggle_mods {
            r.swallow = true;
            self.swallow_key_ups.insert(vk);
            self.mark_swallow();
            if !repeat {
                if self.enabled {
                    self.cancel_gesture(r);
                }
                self.enabled = !self.enabled;
                r.emits.push(Emit { kind: EmitKind::Toggled, x: 0, y: 0, wheel: WheelKind::Ping, enabled: self.enabled });
            }
            return;
        }

        if self.active() {
            if vk == VK_ESCAPE && (self.state == State::Pending || self.state == State::Wheel) {
                r.swallow = true;
                self.swallow_key_ups.insert(vk);
                self.mark_swallow();
                self.cancel_gesture(r);
                return;
            }
            // Caps Lock and custom-key triggers of either wheel would toggle or type: keep them from the OS.
            let own_key = |d: &Self, k: WheelKind| d.is_key_trigger(k) && vk == d.key_trigger_vk(k);
            if own_key(self, WheelKind::Ping) || own_key(self, WheelKind::Emote) {
                r.swallow = true;
                self.swallow_key_ups.insert(vk);
                return;
            }
        }
        // This down passes through, so any pending up-swallow for the key is stale: drop it.
        self.swallow_key_ups.remove(&vk);
    }

    fn on_key_up(&mut self, e: &InputEvent, r: &mut Outcome) {
        let vk = e.vk & 0xFF;
        // Only the gesture's own trigger ends it: the other wheel's key comes and goes freely.
        let held_before = self.held(self.owner);
        self.key_down[vk as usize] = false;
        if !self.held(WheelKind::Ping) && !self.held(WheelKind::Emote) {
            self.foreign_key = false;
        }
        let trigger_released = held_before && !self.held(self.owner) && self.is_trigger_vk(self.owner, vk);

        if self.swallow_key_ups.remove(&vk) {
            r.swallow = true;
        }
        if self.need_mask && (mod_bit(vk) == MOD_ALT || mod_bit(vk) == MOD_WIN) {
            // Swallow the real key-up and replay it after a no-op key, in that order.
            self.need_mask = false;
            r.swallow = true;
            r.injects.push(Inject::key(InjectKind::KeyDown, MASK_VK));
            r.injects.push(Inject::key(InjectKind::KeyUp, MASK_VK));
            r.injects.push(Inject::key(InjectKind::KeyUp, vk));
        }
        if !trigger_released {
            return;
        }
        match self.state {
            State::Pending => {
                if self.click_on(self.owner) {
                    r.emits.push(Emit::point(EmitKind::Click, self.px, self.py, self.owner));
                    self.state = State::SwallowUp;
                } else {
                    // Plain click: re-press now; the real button-up will pass through.
                    r.injects.push(Inject::button(InjectKind::ButtonDown, self.drag_btn));
                    self.state = State::Idle;
                }
            }
            State::Wheel => {
                r.emits.push(Emit::plain(EmitKind::Cancel));
                self.state = State::SwallowUp;
            }
            _ => {}
        }
    }

    fn on_mouse_down(&mut self, e: &InputEvent, r: &mut Outcome) {
        // The matching up was lost, so a fresh drag-button down starts clean.
        if self.state == State::SwallowUp && e.btn == self.drag_btn {
            self.state = State::Idle;
        }
        // A new right-down means the previous right-up never arrived: don't swallow this one's.
        if e.btn == Btn::Right {
            self.swallow_right_up = false;
        }
        if !self.active() {
            return;
        }
        if (self.state == State::Pending || self.state == State::Wheel) && e.btn == Btn::Right {
            r.swallow = true;
            self.swallow_right_up = true;
            self.mark_swallow();
            self.cancel_gesture(r);
            return;
        }
        if self.state != State::Idle {
            return;
        }
        if self.foreign_key {
            return;
        }
        // Exactly one wheel's trigger must claim the press: when both do, pass it through rather than guess.
        let ping = self.starts_gesture(WheelKind::Ping, e.btn);
        let emote = self.starts_gesture(WheelKind::Emote, e.btn);
        if ping != emote {
            // Only a press that would start a gesture pays for the probe.
            let probe = self.fullscreen_probe;
            if self.cfg.game_check && probe.is_some_and(|p| p(e.x, e.y)) {
                return;
            }
            r.swallow = true;
            self.mark_swallow();
            self.state = State::Pending;
            self.owner = if ping { WheelKind::Ping } else { WheelKind::Emote };
            self.drag_btn = e.btn;
            self.px = e.x;
            self.py = e.y;
        }
    }

    fn on_mouse_up(&mut self, e: &InputEvent, r: &mut Outcome) {
        if e.btn == Btn::Right && self.swallow_right_up {
            self.swallow_right_up = false;
            r.swallow = true;
            return;
        }
        if self.state == State::Idle || e.btn != self.drag_btn {
            return;
        }
        r.swallow = true;
        match self.state {
            State::Pending => {
                if self.click_on(self.owner) {
                    r.emits.push(Emit::point(EmitKind::Click, self.px, self.py, self.owner));
                } else {
                    r.injects.push(Inject::button(InjectKind::ButtonDown, self.drag_btn));
                    r.injects.push(Inject::button(InjectKind::ButtonUp, self.drag_btn));
                }
            }
            State::Wheel => {
                r.emits.push(Emit::point(EmitKind::WheelRelease, e.x, e.y, self.owner));
            }
            _ => {}
        }
        self.state = State::Idle;
    }

    fn on_mouse_move(&mut self, e: &InputEvent, r: &mut Outcome) {
        match self.state {
            State::Pending => {
                let dx = f64::from(e.x - self.px);
                let dy = f64::from(e.y - self.py);
                if dx.hypot(dy) > f64::from(self.cfg.drag_threshold_px) {
                    self.state = State::Wheel;
                    r.emits.push(Emit::point(EmitKind::WheelOpen, self.px, self.py, self.owner));
                    r.emits.push(Emit::point(EmitKind::WheelMove, e.x, e.y, self.owner));
                }
            }
            State::Wheel => {
                r.emits.push(Emit::point(EmitKind::WheelMove, e.x, e.y, self.owner));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALT: u32 = 0xA4;
    const CTRL: u32 = 0xA2;
    const KEY_P: u32 = 0x50;
    const KEY_V: u32 = 0x56;
    const ESC: u32 = 0x1B;
    const CAPS: u32 = 0x14;
    const TAB: u32 = 0x09;

    fn kd(vk: u32) -> InputEvent {
        InputEvent { kind: EvKind::KeyDown, vk, ..Default::default() }
    }

    fn ku(vk: u32) -> InputEvent {
        InputEvent { kind: EvKind::KeyUp, vk, ..Default::default() }
    }

    fn md(btn: Btn, x: i32, y: i32) -> InputEvent {
        InputEvent { kind: EvKind::MouseDown, btn, x, y, ..Default::default() }
    }

    fn mu(btn: Btn, x: i32, y: i32) -> InputEvent {
        InputEvent { kind: EvKind::MouseUp, btn, x, y, ..Default::default() }
    }

    fn mv(x: i32, y: i32) -> InputEvent {
        InputEvent { kind: EvKind::MouseMove, x, y, ..Default::default() }
    }

    fn emit(r: &Outcome, kind: EmitKind) -> Option<Emit> {
        r.emits.iter().copied().find(|e| e.kind == kind)
    }

    fn with_trigger(t: Trigger, vk: u32, click_ping: bool) -> Decision {
        let mut d = Decision::new();
        d.set_config(Config { trigger: t, trigger_vk: vk, click_ping, ..Config::default() });
        d
    }

    fn with_emote(t: Trigger, vk: u32, emote_click: bool) -> Decision {
        let mut d = Decision::new();
        d.set_config(Config { emote_trigger: t, emote_trigger_vk: vk, emote_click, ..Config::default() });
        d
    }

    fn open_wheel(d: &mut Decision) {
        d.on_event(&kd(ALT));
        d.on_event(&md(Btn::Left, 100, 100));
        d.on_event(&mv(130, 100));
    }

    #[test]
    fn alt_drag_opens_wheel_and_releases() {
        let mut d = Decision::new();
        assert!(!d.on_event(&kd(ALT)).swallow);
        let r = d.on_event(&md(Btn::Left, 100, 100));
        assert!(r.swallow);
        assert_eq!(d.state(), State::Pending);
        let r = d.on_event(&mv(104, 104));
        assert!(!r.swallow && r.emits.is_empty());
        let r = d.on_event(&mv(120, 100));
        assert!(!r.swallow);
        assert_eq!(d.state(), State::Wheel);
        assert_eq!(r.emits.len(), 2);
        assert_eq!(r.emits[0].kind, EmitKind::WheelOpen);
        assert_eq!((r.emits[0].x, r.emits[0].y), (100, 100));
        assert_eq!(r.emits[1].kind, EmitKind::WheelMove);
        assert_eq!(r.emits[1].x, 120);
        let r = d.on_event(&mu(Btn::Left, 130, 90));
        assert!(r.swallow);
        let rel = emit(&r, EmitKind::WheelRelease).expect("release");
        assert_eq!((rel.x, rel.y), (130, 90));
        assert_eq!(rel.wheel, WheelKind::Ping);
        assert_eq!(d.state(), State::Idle);
    }

    #[test]
    fn plain_click_passes_through() {
        let mut d = Decision::new();
        assert!(!d.on_event(&md(Btn::Left, 1, 1)).swallow);
        assert!(!d.on_event(&mu(Btn::Left, 1, 1)).swallow);
        assert_eq!(d.state(), State::Idle);
    }

    #[test]
    fn alt_click_without_drag_replays_the_click() {
        let mut d = Decision::new();
        d.on_event(&kd(ALT));
        assert!(d.on_event(&md(Btn::Left, 50, 60)).swallow);
        d.on_event(&mv(53, 62));
        let r = d.on_event(&mu(Btn::Left, 53, 62));
        assert!(r.swallow);
        assert!(r.emits.is_empty());
        assert_eq!(
            r.injects,
            vec![
                Inject::button(InjectKind::ButtonDown, Btn::Left),
                Inject::button(InjectKind::ButtonUp, Btn::Left)
            ]
        );
        assert_eq!(d.state(), State::Idle);
    }

    #[test]
    fn click_ping_on_places_a_ping() {
        let mut d = with_trigger(Trigger::Alt, 0, true);
        d.on_event(&kd(ALT));
        d.on_event(&md(Btn::Left, 50, 60));
        let r = d.on_event(&mu(Btn::Left, 51, 60));
        assert!(r.swallow);
        assert!(r.injects.is_empty());
        let click = emit(&r, EmitKind::Click).expect("click");
        assert_eq!((click.x, click.y), (50, 60));
    }

    #[test]
    fn esc_cancels_an_open_wheel() {
        let mut d = Decision::new();
        open_wheel(&mut d);
        let r = d.on_event(&kd(ESC));
        assert!(r.swallow);
        assert!(emit(&r, EmitKind::Cancel).is_some());
        assert_eq!(d.state(), State::SwallowUp);
    }

    #[test]
    fn right_click_cancels_and_swallows_its_up() {
        let mut d = Decision::new();
        open_wheel(&mut d);
        let r = d.on_event(&md(Btn::Right, 200, 200));
        assert!(r.swallow);
        assert!(emit(&r, EmitKind::Cancel).is_some());
        let r = d.on_event(&mu(Btn::Right, 200, 200));
        assert!(r.swallow);
    }

    #[test]
    fn toggle_hotkey_disables_and_enables() {
        let mut d = Decision::new();
        d.on_event(&kd(CTRL));
        d.on_event(&kd(ALT));
        let r = d.on_event(&kd(KEY_P));
        assert!(r.swallow);
        assert!(!emit(&r, EmitKind::Toggled).expect("toggled").enabled);
        assert!(!d.enabled());
        assert!(d.on_event(&ku(KEY_P)).swallow);
        d.on_event(&ku(CTRL));
        assert!(!d.on_event(&md(Btn::Left, 1, 1)).swallow);
        d.on_event(&mu(Btn::Left, 1, 1));
        d.on_event(&kd(CTRL));
        let r = d.on_event(&kd(KEY_P));
        assert!(emit(&r, EmitKind::Toggled).expect("toggled").enabled);
        assert!(d.enabled());
    }

    #[test]
    fn hotkey_autorepeat_toggles_once() {
        let mut d = Decision::new();
        d.on_event(&kd(CTRL));
        d.on_event(&kd(ALT));
        assert!(emit(&d.on_event(&kd(KEY_P)), EmitKind::Toggled).is_some());
        let repeat = d.on_event(&kd(KEY_P));
        assert!(repeat.swallow);
        assert!(emit(&repeat, EmitKind::Toggled).is_none());
        assert!(!d.enabled());
    }

    #[test]
    fn hotkey_ignored_while_suspended() {
        let mut d = Decision::new();
        d.set_suspended(true);
        d.on_event(&kd(CTRL));
        d.on_event(&kd(ALT));
        let r = d.on_event(&kd(KEY_P));
        assert!(!r.swallow && r.emits.is_empty());
        assert!(d.enabled());
        assert!(!d.on_event(&md(Btn::Left, 1, 1)).swallow);
    }

    #[test]
    fn watchdog_cancels_a_stuck_wheel() {
        let mut d = Decision::new();
        open_wheel(&mut d);
        assert!(d.tick(WATCHDOG_MS).emits.is_empty());
        assert!(emit(&d.tick(WATCHDOG_MS + 1), EmitKind::Cancel).is_some());
        assert_eq!(d.state(), State::SwallowUp);
    }

    #[test]
    fn alt_up_during_pending_replays_the_click_after_the_mask() {
        let mut d = Decision::new();
        d.on_event(&kd(ALT));
        d.on_event(&md(Btn::Left, 100, 100));
        let r = d.on_event(&ku(ALT));
        assert!(r.swallow);
        assert_eq!(
            r.injects,
            vec![
                Inject::key(InjectKind::KeyDown, MASK_VK),
                Inject::key(InjectKind::KeyUp, MASK_VK),
                Inject::key(InjectKind::KeyUp, ALT),
                Inject::button(InjectKind::ButtonDown, Btn::Left)
            ]
        );
        assert_eq!(d.state(), State::Idle);
        assert!(!d.on_event(&mu(Btn::Left, 100, 100)).swallow);
    }

    #[test]
    fn alt_up_after_a_gesture_is_masked() {
        let mut d = Decision::new();
        open_wheel(&mut d);
        d.on_event(&mu(Btn::Left, 130, 100));
        let r = d.on_event(&ku(ALT));
        assert!(r.swallow);
        assert_eq!(
            r.injects,
            vec![
                Inject::key(InjectKind::KeyDown, MASK_VK),
                Inject::key(InjectKind::KeyUp, MASK_VK),
                Inject::key(InjectKind::KeyUp, ALT)
            ]
        );
    }

    #[test]
    fn plain_alt_tap_is_not_masked() {
        let mut d = Decision::new();
        d.on_event(&kd(ALT));
        let r = d.on_event(&ku(ALT));
        assert!(!r.swallow && r.injects.is_empty());
    }

    #[test]
    fn mask_off_leaves_alt_up_alone() {
        let mut d = Decision::new();
        d.set_mask_menu_keys(false);
        open_wheel(&mut d);
        d.on_event(&mu(Btn::Left, 130, 100));
        let r = d.on_event(&ku(ALT));
        assert!(!r.swallow && r.injects.is_empty());
    }

    #[test]
    fn caps_lock_trigger_is_kept_from_the_os() {
        let mut d = with_trigger(Trigger::CapsLock, 0, false);
        assert!(d.on_event(&kd(CAPS)).swallow);
        let r = d.on_event(&md(Btn::Left, 100, 100));
        assert!(r.swallow);
        assert_eq!(d.state(), State::Pending);
    }

    #[test]
    fn custom_key_trigger_does_not_type() {
        let mut d = with_trigger(Trigger::CustomVk, KEY_V, false);
        assert!(d.on_event(&kd(KEY_V)).swallow);
        assert!(d.on_event(&ku(KEY_V)).swallow);
    }

    #[test]
    fn both_triggers_held_passes_the_press_through() {
        let mut d = with_emote(Trigger::Ctrl, 0, false);
        d.on_event(&kd(ALT));
        d.on_event(&kd(CTRL));
        assert!(!d.on_event(&md(Btn::Left, 100, 100)).swallow);
        assert_eq!(d.state(), State::Idle);
    }

    #[test]
    fn emote_wheel_opens_on_its_own_trigger() {
        let mut d = with_emote(Trigger::Ctrl, 0, false);
        d.on_event(&kd(CTRL));
        d.on_event(&md(Btn::Left, 100, 100));
        let r = d.on_event(&mv(140, 100));
        assert_eq!(emit(&r, EmitKind::WheelOpen).expect("open").wheel, WheelKind::Emote);
    }

    #[test]
    fn sync_drops_keys_the_os_says_are_up() {
        let mut d = Decision::new();
        d.on_event(&kd(ALT));
        d.sync_key_states(|_| false);
        assert!(!d.on_event(&md(Btn::Left, 5, 5)).swallow);
    }

    #[test]
    fn injected_events_are_ignored() {
        let mut d = Decision::new();
        d.on_event(&kd(ALT));
        let e = InputEvent { kind: EvKind::MouseDown, btn: Btn::Left, self_injected: true, ..Default::default() };
        let r = d.on_event(&e);
        assert!(!r.swallow && r.emits.is_empty());
        assert_eq!(d.state(), State::Idle);
    }

    #[test]
    fn suspend_cancels_but_keeps_the_config() {
        let mut d = Decision::new();
        open_wheel(&mut d);
        let r = d.set_suspended(true);
        assert!(emit(&r, EmitKind::Cancel).is_some());
        assert!(d.enabled());
        d.set_suspended(false);
        assert!(d.on_event(&md(Btn::Left, 5, 5)).swallow);
        assert_eq!(d.state(), State::Pending);
    }

    #[test]
    fn right_up_lost_then_new_right_down_is_not_swallowed() {
        let mut d = Decision::new();
        open_wheel(&mut d);
        d.on_event(&md(Btn::Right, 200, 200));
        d.on_event(&md(Btn::Right, 201, 201));
        assert!(!d.on_event(&mu(Btn::Right, 201, 201)).swallow);
    }

    fn always_fullscreen(_: i32, _: i32) -> bool {
        true
    }

    fn never_fullscreen(_: i32, _: i32) -> bool {
        false
    }

    #[test]
    fn another_programs_chord_lets_clicks_through_until_the_trigger_is_released() {
        let mut d = Decision::new();
        d.on_event(&kd(ALT));
        d.on_event(&kd(TAB));
        assert!(!d.on_event(&md(Btn::Left, 100, 100)).swallow);
        assert_eq!(d.state(), State::Idle);
        d.on_event(&ku(TAB));
        assert!(!d.on_event(&md(Btn::Left, 100, 100)).swallow);
        d.on_event(&ku(ALT));
        d.on_event(&kd(ALT));
        assert!(d.on_event(&md(Btn::Left, 100, 100)).swallow);
    }

    #[test]
    fn fullscreen_app_keeps_the_click() {
        let mut d = Decision::new();
        d.set_config(Config { game_check: true, ..Config::default() });
        d.set_fullscreen_probe(always_fullscreen);
        d.on_event(&kd(ALT));
        assert!(!d.on_event(&md(Btn::Left, 100, 100)).swallow);
        assert_eq!(d.state(), State::Idle);

        let mut d = Decision::new();
        d.set_config(Config { game_check: true, ..Config::default() });
        d.set_fullscreen_probe(never_fullscreen);
        d.on_event(&kd(ALT));
        assert!(d.on_event(&md(Btn::Left, 100, 100)).swallow);
    }

    #[test]
    fn fullscreen_check_off_ignores_the_probe() {
        let mut d = Decision::new();
        d.set_config(Config { game_check: false, ..Config::default() });
        d.set_fullscreen_probe(always_fullscreen);
        d.on_event(&kd(ALT));
        assert!(d.on_event(&md(Btn::Left, 100, 100)).swallow);
    }
}
