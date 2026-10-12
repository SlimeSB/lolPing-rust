use std::ffi::c_void;
use std::io;
use std::ptr::NonNull;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use objc2_core_foundation::{
    kCFRunLoopCommonModes, CFAbsoluteTimeGetCurrent, CFMachPort, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext,
    CGPoint,
};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventFlags, CGEventMask, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
    CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy, CGEventType, CGMouseButton,
};

use crate::display::DisplayMap;
use crate::input::decision::{Btn, Config, Decision, Emit, EvKind, InjectKind, InputEvent, Outcome};

use super::macinput::{is_modifier_vk, key_code_to_vk, modifier_held, modifier_transitions, vk_to_key_code, MODIFIER_FLAGS};

/// "LPIN" in kCGEventSourceUserData marks our own posted events.
const SELF_MARKER: i64 = 0x4C50_494E;
const FRAME: Duration = Duration::from_millis(16);
const MAPS_AGE: Duration = Duration::from_secs(1);

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

/// Taps the session's events on a thread of its own and reports once the tap is in place.
pub fn spawn(sink: Sender<Emit>, ready: Sender<Result<u32, String>>) -> io::Result<HookHandle> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (up_tx, up_rx) = std::sync::mpsc::channel();
    let join = std::thread::Builder::new().name("input-hook".into()).spawn(move || run(sink, ready, rx, up_tx))?;
    let _ = up_rx.recv();
    Ok(HookHandle { tx, join: Some(join) })
}

struct Context {
    decision: Decision,
    sink: Sender<Emit>,
    commands: Receiver<HookCommand>,
    tap: Option<objc2_core_foundation::CFRetained<CFMachPort>>,
    maps: Vec<DisplayMap>,
    maps_at: Instant,
    flags: u64,
    start: Instant,
}

fn run(sink: Sender<Emit>, ready: Sender<Result<u32, String>>, commands: Receiver<HookCommand>, up: Sender<()>) {
    let context = Box::into_raw(Box::new(Context {
        decision: Decision::new(),
        sink,
        commands,
        tap: None,
        maps: crate::display::displays(),
        maps_at: Instant::now(),
        flags: 0,
        start: Instant::now(),
    }));
    unsafe { (*context).decision.set_mask_menu_keys(false) };
    let mask =         bit(CGEventType::KeyDown)
            | bit(CGEventType::KeyUp)
            | bit(CGEventType::FlagsChanged)
            | bit(CGEventType::LeftMouseDown)
            | bit(CGEventType::LeftMouseUp)
            | bit(CGEventType::RightMouseDown)
            | bit(CGEventType::RightMouseUp)
            | bit(CGEventType::OtherMouseDown)
            | bit(CGEventType::OtherMouseUp)
            | bit(CGEventType::MouseMoved)
            | bit(CGEventType::LeftMouseDragged)
            | bit(CGEventType::RightMouseDragged)
            | bit(CGEventType::OtherMouseDragged);
    let tap = unsafe {
        CGEvent::tap_create(
            CGEventTapLocation::SessionEventTap,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            mask,
            Some(tap_callback),
            context.cast(),
        )
    };
    let Some(tap) = tap else {
        let _ = up.send(());
        let _ = ready.send(Err("Can't read input: lolPing needs Accessibility permission".into()));
        drop(unsafe { Box::from_raw(context) });
        return;
    };
    let Some(loop_) = CFRunLoop::current() else {
        let _ = up.send(());
        let _ = ready.send(Err("no run loop".into()));
        drop(unsafe { Box::from_raw(context) });
        return;
    };
    let source = CFMachPort::new_run_loop_source(None, Some(&tap), 0);
    unsafe {
        CFRunLoop::add_source(&loop_, source.as_deref(), kCFRunLoopCommonModes);
        CGEvent::tap_enable(&tap, true);
    }
    let mut timer_context = CFRunLoopTimerContext {
        version: 0,
        info: context.cast(),
        retain: None,
        release: None,
        copyDescription: None,
    };
    let timer = unsafe {
        CFRunLoopTimer::new(
            None,
            CFAbsoluteTimeGetCurrent() + 1.0,
            FRAME.as_secs_f64(),
            0,
            0,
            Some(timer_callback),
            &mut timer_context,
        )
    };
    unsafe { CFRunLoop::add_timer(&loop_, timer.as_deref(), kCFRunLoopCommonModes) };
    unsafe { (*context).tap = Some(tap) };
    resync(unsafe { &mut *context });
    let _ = up.send(());
    let _ = ready.send(Ok(0));
    CFRunLoop::run();
    unsafe {
        if let Some(tap) = &(*context).tap {
            CGEvent::tap_enable(tap, false);
        }
        if let Some(timer) = &timer {
            CFRunLoopTimer::invalidate(timer);
        }
        CFRunLoop::remove_source(&loop_, source.as_deref(), kCFRunLoopCommonModes);
        if let Some(tap) = &(*context).tap {
            CFMachPort::invalidate(tap);
        }
    }
    drop(unsafe { Box::from_raw(context) });
}

fn bit(kind: CGEventType) -> CGEventMask {
    1u64 << kind.0
}

fn now_ms(context: &Context) -> u64 {
    context.start.elapsed().as_millis() as u64
}

/// Turns a modifier-flag change into the key events Decision expects: mouse events carry the flags too, which keeps the state right while Secure Event Input hides keyboard events from the tap.
fn feed_flags(context: &mut Context, flags: u64, time_ms: u64) {
    let flags = flags & MODIFIER_FLAGS;
    if flags == context.flags {
        return;
    }
    for step in modifier_transitions(context.flags, flags) {
        let kind = if step.down { EvKind::KeyDown } else { EvKind::KeyUp };
        let event = InputEvent { kind, vk: step.vk, time_ms, ..Default::default() };
        let outcome = context.decision.on_event(&event);
        apply(context, outcome);
    }
    context.flags = flags;
}

fn resync(context: &mut Context) {
    let flags = source_flags();
    feed_flags(context, flags, now_ms(context));
    context.decision.sync_key_states(is_key_down);
}

fn source_flags() -> u64 {
    CGEventSource::flags_state(CGEventSourceStateID::CombinedSessionState).0
}

fn is_key_down(vk: u32) -> bool {
    let state = CGEventSourceStateID::CombinedSessionState;
    if is_modifier_vk(vk) {
        return modifier_held(source_flags(), vk);
    }
    vk_to_key_code(vk).is_some_and(|code| CGEventSource::key_state(state, code))
}

fn cursor_location() -> CGPoint {
    match CGEvent::new(None) {
        Some(event) => CGEvent::location(Some(&event)),
        None => CGPoint { x: 0.0, y: 0.0 },
    }
}

fn post_button(context: &Context, btn: Btn, down: bool) {
    let (kind, button) = match btn {
        Btn::Left => (mouse_type(down, CGEventType::LeftMouseDown, CGEventType::LeftMouseUp), CGMouseButton::Left),
        Btn::Right => (mouse_type(down, CGEventType::RightMouseDown, CGEventType::RightMouseUp), CGMouseButton::Right),
        Btn::Middle => (mouse_type(down, CGEventType::OtherMouseDown, CGEventType::OtherMouseUp), CGMouseButton::Center),
        Btn::X1 => (mouse_type(down, CGEventType::OtherMouseDown, CGEventType::OtherMouseUp), CGMouseButton(3)),
        Btn::X2 => (mouse_type(down, CGEventType::OtherMouseDown, CGEventType::OtherMouseUp), CGMouseButton(4)),
        Btn::None => return,
    };
    let Some(event) = CGEvent::new_mouse_event(None, kind, cursor_location(), button) else {
        return;
    };
    CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventButtonNumber, button.0 as i64);
    CGEvent::set_integer_value_field(Some(&event), CGEventField::MouseEventClickState, 1);
    CGEvent::set_flags(Some(&event), CGEventFlags(context.flags));
    CGEvent::set_integer_value_field(Some(&event), CGEventField::EventSourceUserData, SELF_MARKER);
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&event));
}

fn mouse_type(down: bool, press: CGEventType, release: CGEventType) -> CGEventType {
    if down {
        press
    } else {
        release
    }
}

/// Reports emits and replays buttons, then answers whether the event has to be swallowed.
fn apply(context: &Context, outcome: Outcome) -> bool {
    for emit in outcome.emits {
        let _ = context.sink.send(emit);
    }
    for inject in outcome.injects {
        match inject.kind {
            InjectKind::ButtonDown => post_button(context, inject.btn, true),
            InjectKind::ButtonUp => post_button(context, inject.btn, false),
            InjectKind::KeyDown | InjectKind::KeyUp => {}
        }
    }
    outcome.swallow
}

unsafe extern "C-unwind" fn tap_callback(
    _proxy: CGEventTapProxy,
    kind: CGEventType,
    event: NonNull<CGEvent>,
    user_info: *mut c_void,
) -> *mut CGEvent {
    let context = unsafe { &mut *user_info.cast::<Context>() };
    if kind == CGEventType::TapDisabledByTimeout || kind == CGEventType::TapDisabledByUserInput {
        if let Some(tap) = &context.tap {
            CGEvent::tap_enable(tap, true);
        }
        resync(context);
        return event.as_ptr();
    }
    let event_ref = unsafe { event.as_ref() };
    if CGEvent::integer_value_field(Some(event_ref), CGEventField::EventSourceUserData) == SELF_MARKER {
        return event.as_ptr();
    }
    let time_ms = now_ms(context);
    feed_flags(context, CGEvent::flags(Some(event_ref)).0, time_ms);
    if kind == CGEventType::FlagsChanged {
        return event.as_ptr();
    }
    if kind == CGEventType::KeyDown || kind == CGEventType::KeyUp {
        let code = CGEvent::integer_value_field(Some(event_ref), CGEventField::KeyboardEventKeycode) as u16;
        let vk = key_code_to_vk(code);
        if vk == 0 {
            return event.as_ptr();
        }
        let event_kind = if kind == CGEventType::KeyDown { EvKind::KeyDown } else { EvKind::KeyUp };
        let input = InputEvent { kind: event_kind, vk, time_ms, ..Default::default() };
        let outcome = context.decision.on_event(&input);
        return if apply(context, outcome) { std::ptr::null_mut() } else { event.as_ptr() };
    }
    let point = to_pixels(context, CGEvent::location(Some(event_ref)));
    let mut input = InputEvent { x: point.0, y: point.1, time_ms, ..Default::default() };
    match kind {
        CGEventType::MouseMoved
        | CGEventType::LeftMouseDragged
        | CGEventType::RightMouseDragged
        | CGEventType::OtherMouseDragged => input.kind = EvKind::MouseMove,
        CGEventType::LeftMouseDown | CGEventType::LeftMouseUp => {
            input.kind = button_kind(kind, CGEventType::LeftMouseDown);
            input.btn = Btn::Left;
        }
        CGEventType::RightMouseDown | CGEventType::RightMouseUp => {
            input.kind = button_kind(kind, CGEventType::RightMouseDown);
            input.btn = Btn::Right;
        }
        CGEventType::OtherMouseDown | CGEventType::OtherMouseUp => {
            let number = CGEvent::integer_value_field(Some(event_ref), CGEventField::MouseEventButtonNumber);
            input.btn = match number {
                2 => Btn::Middle,
                3 => Btn::X1,
                4 => Btn::X2,
                _ => return event.as_ptr(),
            };
            input.kind = button_kind(kind, CGEventType::OtherMouseDown);
        }
        _ => return event.as_ptr(),
    }
    if input.kind == EvKind::MouseDown {
        context.decision.sync_key_states(is_key_down);
    }
    let outcome = context.decision.on_event(&input);
    if apply(context, outcome) {
        std::ptr::null_mut()
    } else {
        event.as_ptr()
    }
}

fn button_kind(kind: CGEventType, down: CGEventType) -> EvKind {
    if kind == down {
        EvKind::MouseDown
    } else {
        EvKind::MouseUp
    }
}

/// The tap hands out global points, and the overlay works in the backing pixels the display maps carry.
fn to_pixels(context: &Context, point: CGPoint) -> (i32, i32) {
    crate::display::dip_to_physical(point.x, point.y, &context.maps)
        .unwrap_or((point.x.round() as i32, point.y.round() as i32))
}

unsafe extern "C-unwind" fn timer_callback(_timer: *mut CFRunLoopTimer, user_info: *mut c_void) {
    let context = unsafe { &mut *user_info.cast::<Context>() };
    if context.maps_at.elapsed() >= MAPS_AGE {
        context.maps = crate::display::displays();
        context.maps_at = Instant::now();
    }
    let mut stop = false;
    loop {
        match context.commands.try_recv() {
            Ok(HookCommand::Shutdown) => {
                stop = true;
                break;
            }
            Ok(HookCommand::Config(config)) => {
                let outcome = context.decision.set_config(config);
                apply(context, outcome);
            }
            Ok(HookCommand::Enabled(on)) => {
                let outcome = context.decision.set_enabled(on);
                apply(context, outcome);
            }
            Ok(HookCommand::Suspend(on)) => {
                let outcome = context.decision.set_suspended(on);
                apply(context, outcome);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => break,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                stop = true;
                break;
            }
        }
    }
    let time_ms = now_ms(context);
    resync(context);
    let outcome = context.decision.tick(time_ms);
    apply(context, outcome);
    if stop {
        if let Some(loop_) = CFRunLoop::current() {
            CFRunLoop::stop(&loop_);
        }
    }
}
