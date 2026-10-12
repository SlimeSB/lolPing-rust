use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::LazyLock;
use std::time::Instant;

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
    MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
    PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WH_MOUSE_LL, WM_APP, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_TIMER, WM_USER, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1, XBUTTON2,
};

use crate::input::decision::{Btn, Config, Decision, Emit, EvKind, Inject, InjectKind, InputEvent, Outcome};

/// "LPIN" in dwExtraInfo marks our own SendInput events.
pub const SELF_MARKER: usize = 0x4C50_494E;

const WM_COMMAND: u32 = WM_APP + 1;
const WM_INJECTS: u32 = WM_APP + 2;
const WATCHDOG_INTERVAL_MS: u32 = 1000;

static HOOK_THREAD: AtomicU32 = AtomicU32::new(0);

pub enum HookCommand {
    Config(Config),
    Enabled(bool),
    Suspend(bool),
    Shutdown,
}

struct HookState {
    decision: Decision,
    sink: Sender<Emit>,
}

thread_local! {
    static STATE: RefCell<Option<HookState>> = const { RefCell::new(None) };
}

pub struct HookHandle {
    thread_id: u32,
    join: Option<std::thread::JoinHandle<()>>,
}

impl HookHandle {
    pub fn send(&self, cmd: HookCommand) -> bool {
        let raw = Box::into_raw(Box::new(cmd));
        let posted = unsafe { PostThreadMessageW(self.thread_id, WM_COMMAND, WPARAM(0), LPARAM(raw as isize)) };
        if posted.is_err() {
            // SAFETY: the message was never posted, so nobody else owns the box.
            drop(unsafe { Box::from_raw(raw) });
            return false;
        }
        true
    }

    pub fn shutdown(mut self) {
        self.send(HookCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Installs the low-level hooks on a thread of its own and reports `Ok(thread id)` once they are in place.
pub fn spawn(sink: Sender<Emit>, ready: Sender<Result<u32, String>>) -> std::io::Result<HookHandle> {
    let (id_tx, id_rx) = std::sync::mpsc::channel();
    let join = std::thread::Builder::new().name("input-hook".into()).spawn(move || {
        run(sink, ready, id_tx);
    })?;
    let thread_id = id_rx.recv().unwrap_or(0);
    Ok(HookHandle { thread_id, join: Some(join) })
}

fn now_ms() -> u64 {
    static START: LazyLock<Instant> = LazyLock::new(Instant::now);
    START.elapsed().as_millis() as u64
}

fn is_key_down(vk: u32) -> bool {
    unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
}

fn is_extended_vk(vk: u32) -> bool {
    matches!(vk, 0xA5 | 0xA3 | 0x5B | 0x5C | 0x2D | 0x2E | 0x24 | 0x23 | 0x21 | 0x22 | 0x25 | 0x26 | 0x27 | 0x28)
}

fn send_injects(injects: &[Inject]) {
    if injects.is_empty() {
        return;
    }
    let mut inputs: Vec<INPUT> = Vec::with_capacity(injects.len());
    for i in injects {
        let mut input = INPUT::default();
        match i.kind {
            InjectKind::ButtonDown | InjectKind::ButtonUp => {
                let down = i.kind == InjectKind::ButtonDown;
                let (flags, data) = match (i.btn, down) {
                    (Btn::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                    (Btn::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
                    (Btn::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                    (Btn::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
                    (Btn::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                    (Btn::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                    (Btn::X1, true) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON1)),
                    (Btn::X1, false) => (MOUSEEVENTF_XUP, u32::from(XBUTTON1)),
                    (Btn::X2, true) => (MOUSEEVENTF_XDOWN, u32::from(XBUTTON2)),
                    (Btn::X2, false) => (MOUSEEVENTF_XUP, u32::from(XBUTTON2)),
                    (Btn::None, _) => continue,
                };
                input.r#type = INPUT_MOUSE;
                input.Anonymous.mi =
                    MOUSEINPUT { dx: 0, dy: 0, mouseData: data, dwFlags: flags, time: 0, dwExtraInfo: SELF_MARKER };
            }
            InjectKind::KeyDown | InjectKind::KeyUp => {
                let mut flags = if i.kind == InjectKind::KeyUp { KEYEVENTF_KEYUP } else { Default::default() };
                if is_extended_vk(i.vk) {
                    flags |= KEYEVENTF_EXTENDEDKEY;
                }
                input.r#type = INPUT_KEYBOARD;
                input.Anonymous.ki = KEYBDINPUT {
                    wVk: VIRTUAL_KEY(i.vk as u16),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: SELF_MARKER,
                };
            }
        }
        inputs.push(input);
    }
    if !inputs.is_empty() {
        unsafe {
            SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        }
    }
}

/// Injected input is posted back to the hook thread: SendInput inside a callback waits on the hook chain running it.
fn post_injects(injects: Vec<Inject>) {
    if injects.is_empty() {
        return;
    }
    let raw = Box::into_raw(Box::new(injects));
    let tid = HOOK_THREAD.load(Ordering::Relaxed);
    let posted = tid != 0 && unsafe { PostThreadMessageW(tid, WM_INJECTS, WPARAM(0), LPARAM(raw as isize)) }.is_ok();
    if !posted {
        // SAFETY: the message was never posted, so nobody else owns the box.
        drop(unsafe { Box::from_raw(raw) });
    }
}

fn apply(outcome: Outcome, sink: Option<&Sender<Emit>>) -> bool {
    if let Some(sink) = sink {
        for e in &outcome.emits {
            let _ = sink.send(*e);
        }
    }
    post_injects(outcome.injects);
    outcome.swallow
}

fn with_state<R>(f: impl FnOnce(&mut HookState) -> R) -> Option<(R, Sender<Emit>)> {
    STATE.with(|cell| {
        let mut guard = cell.borrow_mut();
        let st = guard.as_mut()?;
        let sink = st.sink.clone();
        Some((f(st), sink))
    })
}

fn on_event(event: InputEvent) -> bool {
    let Some((outcome, sink)) = with_state(|st| st.decision.on_event(&event)) else {
        return false;
    };
    apply(outcome, Some(&sink))
}

fn on_command(cmd: HookCommand) {
    let Some((outcome, sink)) = with_state(|st| match cmd {
        HookCommand::Config(c) => st.decision.set_config(c),
        HookCommand::Enabled(on) => st.decision.set_enabled(on),
        HookCommand::Suspend(on) => st.decision.set_suspended(on),
        HookCommand::Shutdown => Outcome::default(),
    }) else {
        return;
    };
    apply(outcome, Some(&sink));
}

fn on_tick() {
    let Some((outcome, sink)) = with_state(|st| {
        st.decision.sync_key_states(is_key_down);
        st.decision.tick(now_ms())
    }) else {
        return;
    };
    apply(outcome, Some(&sink));
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: the system passes an MSLLHOOKSTRUCT for WH_MOUSE_LL.
        let m = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let message = wparam.0 as u32;
        let mut e = InputEvent { x: m.pt.x, y: m.pt.y, time_ms: now_ms(), ..Default::default() };
        e.self_injected = m.flags & LLMHF_INJECTED != 0 && m.dwExtraInfo == SELF_MARKER;
        let known = match message {
            WM_MOUSEMOVE => {
                e.kind = EvKind::MouseMove;
                true
            }
            WM_LBUTTONDOWN | WM_LBUTTONUP => {
                e.kind = if message == WM_LBUTTONDOWN { EvKind::MouseDown } else { EvKind::MouseUp };
                e.btn = Btn::Left;
                true
            }
            WM_RBUTTONDOWN | WM_RBUTTONUP => {
                e.kind = if message == WM_RBUTTONDOWN { EvKind::MouseDown } else { EvKind::MouseUp };
                e.btn = Btn::Right;
                true
            }
            WM_MBUTTONDOWN | WM_MBUTTONUP => {
                e.kind = if message == WM_MBUTTONDOWN { EvKind::MouseDown } else { EvKind::MouseUp };
                e.btn = Btn::Middle;
                true
            }
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                e.kind = if message == WM_XBUTTONDOWN { EvKind::MouseDown } else { EvKind::MouseUp };
                e.btn = if (m.mouseData >> 16) as u16 == XBUTTON1 { Btn::X1 } else { Btn::X2 };
                true
            }
            _ => false,
        };
        if known && e.kind == EvKind::MouseDown {
            with_state(|st| st.decision.sync_key_states(is_key_down));
        }
        if known && on_event(e) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn key_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: the system passes a KBDLLHOOKSTRUCT for WH_KEYBOARD_LL.
        let k = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let down = wparam.0 as u32 == WM_KEYDOWN || wparam.0 as u32 == WM_SYSKEYDOWN;
        let mut e = InputEvent {
            kind: if down { EvKind::KeyDown } else { EvKind::KeyUp },
            vk: k.vkCode,
            time_ms: now_ms(),
            ..Default::default()
        };
        e.self_injected = k.flags.contains(LLKHF_INJECTED) && k.dwExtraInfo == SELF_MARKER;
        if on_event(e) {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn run(sink: Sender<Emit>, ready: Sender<Result<u32, String>>, id_tx: Sender<u32>) {
    STATE.with(|cell| {
        *cell.borrow_mut() = Some(HookState { decision: Decision::new(), sink });
    });

    let mut msg = MSG::default();
    let (mouse_hook, key_hook, thread_id) = unsafe {
        // SAFETY: the thread needs a message queue before anything posts to it.
        let _ = PeekMessageW(&mut msg, None, WM_USER, WM_USER, PM_NOREMOVE);
        let module = GetModuleHandleW(None).unwrap_or_default();
        let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), Some(HINSTANCE(module.0)), 0);
        let key = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), Some(HINSTANCE(module.0)), 0);
        (mouse, key, GetCurrentThreadId())
    };

    let (mouse_hook, key_hook) = match (mouse_hook, key_hook) {
        (Ok(m), Ok(k)) => (m, k),
        (m, k) => {
            unsafe {
                if let Ok(h) = m {
                    let _ = UnhookWindowsHookEx(h);
                }
                if let Ok(h) = k {
                    let _ = UnhookWindowsHookEx(h);
                }
            }
            let _ = id_tx.send(thread_id);
            let _ = ready.send(Err("SetWindowsHookEx failed".into()));
            return;
        }
    };

    unsafe { SetTimer(None, 0, WATCHDOG_INTERVAL_MS, None) };
    HOOK_THREAD.store(thread_id, Ordering::Relaxed);
    let _ = id_tx.send(thread_id);
    let _ = ready.send(Ok(thread_id));

    loop {
        let got = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if !got.as_bool() {
            break;
        }
        match msg.message {
            WM_TIMER => on_tick(),
            WM_INJECTS => {
                // SAFETY: only `post_injects` posts this message, and it hands the box over.
                let injects = unsafe { *Box::from_raw(msg.lParam.0 as *mut Vec<Inject>) };
                send_injects(&injects);
            }
            WM_COMMAND => {
                // SAFETY: only `HookHandle::send` posts this message, and it hands the box over.
                let cmd = unsafe { *Box::from_raw(msg.lParam.0 as *mut HookCommand) };
                if matches!(cmd, HookCommand::Shutdown) {
                    break;
                }
                on_command(cmd);
            }
            _ => {}
        }
    }

    unsafe {
        let _ = UnhookWindowsHookEx(mouse_hook);
        let _ = UnhookWindowsHookEx(key_hook);
    }
    HOOK_THREAD.store(0, Ordering::Relaxed);
    STATE.with(|cell| {
        *cell.borrow_mut() = None;
    });
}
