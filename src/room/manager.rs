use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;

use super::code::{format_room_code, make_room_code, parse_room_code, ParsedRoomCode};
use super::crypto::{derive_room_keys, open, seal, RoomKeys};
use super::protocol::{decode_message, encode_message, is_older_than, RoomMessage, Status, PROTOCOL_VERSION};
use super::rate::{Clock, RateLimiter};
use super::transport::{Link, TransportEvent, TransportHandle, TransportKind};

pub const MAX_MEMBERS: usize = 8;
pub const RELAY_PING_RATE: f64 = 5.0;
pub const RELAY_PING_BURST: f64 = 10.0;

const SETTLE_MS: i64 = 3000;
const SETTLE_CAP_MS: i64 = 15_000;
const TICK_MS: i64 = 1000;
const SHARED_PRESENCE_MS: i64 = 1000;
const SLEEP_GAP_MS: i64 = 5000;
const PRESENCE_MS: [i64; 2] = [2000, 10_000];
const ALIVE_MS: [i64; 2] = [6000, 25_000];
const DROP_MS: [i64; 2] = [15_000, 30_000];
const MAX_SKEW_MS: i64 = 10 * 60 * 1000;
const UNREACHABLE_MS: i64 = 20_000;
const LONELY_MS: i64 = 20_000;
const STALE_STRIKES: u32 = 3;
const SKEW_HINT_MS: i64 = 30_000;
const EMOTES_SINCE: &str = "0.5.0";

pub struct Profile {
    pub name: String,
    pub color: u32,
    pub limit: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Path {
    Lan,
    Internet,
    Connecting,
    Unreachable,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    Idle,
    Joining,
    Active,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    pub peer: String,
    pub name: String,
    pub color: u32,
    pub status: String,
    pub path: Path,
    pub needs_update: bool,
    pub no_emotes: bool,
    pub muted: bool,
    pub me: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RoomState {
    pub phase: Phase,
    pub code: Option<String>,
    pub members: Vec<Member>,
    pub lan: Link,
    pub internet: Link,
    pub internet_allowed: bool,
    pub paused: bool,
    pub room_muted: bool,
    pub lonely: bool,
    pub clock_skew: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub name: String,
    pub color: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Toast {
    JoinedRoom { code: String },
    Full,
    Joined { name: String },
    Left { name: String },
    Lost { name: String },
}

#[derive(Clone, Debug)]
pub enum RoomEvent {
    State(Arc<RoomState>),
    Ping { number: u32, fx: f64, fy: f64, id: String, tag: Tag },
    Emote { number: u32, fx: f64, fy: f64, reference: String, peer: String, tag: Tag },
    Toast(Toast),
    SaveCode(Option<String>),
}

pub enum RoomCommand {
    Create,
    Join(String),
    Leave,
    Mute { peer: String, on: bool },
    SetInternet(bool),
    SetFlags { paused: bool, room_muted: bool },
    SendPing { number: u32, fx: f64, fy: f64, id: String },
    SendEmote { number: u32, fx: f64, fy: f64, reference: String },
    Profile(Profile),
    RetryInternet,
    Wake,
    Quit,
}

// A transport as the manager sees it: the frozen handle plus the test double.
trait Net: Send {
    fn kind(&self) -> TransportKind;
    fn shared(&self) -> bool;
    fn start(&self, keys: RoomKeys);
    fn send_to(&self, peer: &str, bytes: &[u8]);
    fn broadcast(&self, bytes: &[u8]);
    fn stop(&self);
    fn wake(&self);
    fn retry(&self);
    fn shutdown(self: Box<Self>);
}

impl Net for TransportHandle {
    fn kind(&self) -> TransportKind {
        self.kind
    }
    fn shared(&self) -> bool {
        self.shared
    }
    fn start(&self, keys: RoomKeys) {
        TransportHandle::start(self, keys)
    }
    fn send_to(&self, peer: &str, bytes: &[u8]) {
        TransportHandle::send_to(self, peer, bytes)
    }
    fn broadcast(&self, bytes: &[u8]) {
        TransportHandle::broadcast(self, bytes)
    }
    fn stop(&self) {
        TransportHandle::stop(self)
    }
    fn wake(&self) {
        TransportHandle::wake(self)
    }
    fn retry(&self) {
        TransportHandle::retry(self)
    }
    fn shutdown(self: Box<Self>) {
        TransportHandle::shutdown(*self)
    }
}

enum Input {
    Command(RoomCommand),
    Event { tag: u64, ev: TransportEvent },
}

struct Route {
    key: String,
    seen_at: i64,
}

struct MemberCore {
    peer: String,
    name: String,
    color: u32,
    status: String,
    needs_update: bool,
    no_emotes: bool,
    muted: bool,
    first_seen: i64,
    via_internet: bool,
    paths: [Option<Route>; 2],
}

struct Slot {
    tag: u64,
    kind: TransportKind,
    shared: bool,
    link: Link,
    active: bool,
    net: Box<dyn Net>,
    // Only kept when nothing pumps it: the test harness drives events by hand.
    ev: Option<Receiver<TransportEvent>>,
}

enum Verdict {
    Ok,
    Stale,
    Replay,
    Gone,
}

#[derive(Default)]
struct ReplayGuard {
    peers: HashMap<String, (u64, u64)>,
    gone: HashSet<String>,
}

impl ReplayGuard {
    fn accept(&mut self, peer: &str, seq: u64, ts: i64, now: i64) -> Verdict {
        if self.gone.contains(peer) {
            return Verdict::Gone;
        }
        if (ts - now).abs() > MAX_SKEW_MS {
            return Verdict::Stale;
        }
        match self.peers.get_mut(peer) {
            None => {
                self.peers.insert(peer.to_string(), (seq, 1));
                Verdict::Ok
            }
            Some((max, bits)) => {
                if seq > *max {
                    let shift = seq - *max;
                    *bits = if shift >= 64 { 1 } else { (*bits << shift) | 1 };
                    *max = seq;
                    Verdict::Ok
                } else {
                    let back = *max - seq;
                    if back >= 64 {
                        return Verdict::Replay;
                    }
                    let bit = 1u64 << back;
                    if *bits & bit != 0 {
                        return Verdict::Replay;
                    }
                    *bits |= bit;
                    Verdict::Ok
                }
            }
        }
    }

    fn mark_gone(&mut self, peer: &str) {
        self.gone.insert(peer.to_string());
        self.peers.remove(peer);
    }
}

enum Action {
    Ping { id: String, d: u32, x: f64, y: f64 },
    Emote { reference: String, d: u32, x: f64, y: f64 },
}

fn ki(kind: TransportKind) -> usize {
    match kind {
        TransportKind::Lan => 0,
        TransportKind::Internet => 1,
    }
}

fn kind_key(kind: TransportKind) -> &'static str {
    match kind {
        TransportKind::Lan => "lan",
        TransportKind::Internet => "internet",
    }
}

fn room_keys_copy(keys: &RoomKeys) -> RoomKeys {
    RoomKeys { msg_key: keys.msg_key, topic: keys.topic.clone() }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    getrandom::fill(&mut buf).expect("os randomness");
    buf
}

// Binds the LAN socket; None while another process holds the port, retried on every room start.
type LanFactory = Box<dyn FnMut(Sender<TransportEvent>) -> Option<TransportHandle> + Send>;

struct Core {
    events: Sender<RoomEvent>,
    cell: Arc<Mutex<Arc<RoomState>>>,
    // Where a pump thread forwards transport events; None in tests, which pump by hand.
    mux: Option<Sender<Input>>,
    lan_factory: Option<LanFactory>,
    keys_override: Option<RoomKeys>,
    profile: Profile,
    app_version: String,
    self_id: String,
    phase: Phase,
    code: Option<String>,
    keys: Option<RoomKeys>,
    seq: u64,
    members: Vec<MemberCore>,
    answered: HashSet<String>,
    stale_strikes: HashMap<String, (u32, i64)>,
    alone_since: Option<i64>,
    guard: ReplayGuard,
    limiter: RateLimiter,
    relay_cap: RateLimiter,
    paused: bool,
    room_muted: bool,
    last_presence: [Option<i64>; 2],
    last_tick: i64,
    woke_at: i64,
    settle_at: Option<i64>,
    settle_short: bool,
    joined_at: i64,
    last_state: Option<Arc<RoomState>>,
    internet_allowed: bool,
    nets: Vec<Slot>,
    shared_at: [Option<i64>; 2],
    next_tag: u64,
    quit: bool,
}

impl Core {
    #[allow(clippy::too_many_arguments)]
    fn new(
        events: Sender<RoomEvent>,
        cell: Arc<Mutex<Arc<RoomState>>>,
        nets: Vec<(Box<dyn Net>, Receiver<TransportEvent>)>,
        mux: Option<Sender<Input>>,
        lan_factory: Option<LanFactory>,
        keys_override: Option<RoomKeys>,
        clock: Clock,
        profile: Profile,
        app_version: String,
        internet_allowed: bool,
    ) -> Core {
        let mut core = Core {
            events,
            cell,
            mux,
            lan_factory,
            keys_override,
            profile,
            app_version,
            self_id: String::new(),
            phase: Phase::Idle,
            code: None,
            keys: None,
            seq: 0,
            members: Vec::new(),
            answered: HashSet::new(),
            stale_strikes: HashMap::new(),
            alone_since: None,
            guard: ReplayGuard::default(),
            limiter: RateLimiter::new(clock.clone()),
            relay_cap: RateLimiter::new(clock),
            paused: false,
            room_muted: false,
            last_presence: [None; 2],
            last_tick: 0,
            woke_at: 0,
            settle_at: None,
            settle_short: false,
            joined_at: 0,
            last_state: None,
            internet_allowed,
            nets: Vec::new(),
            shared_at: [None; 2],
            next_tag: 1,
            quit: false,
        };
        for (net, receiver) in nets {
            core.add_net(net, receiver);
        }
        core
    }

    #[cfg(test)]
    fn state(&self) -> Arc<RoomState> {
        self.cell.lock().clone()
    }

    fn room_keys(&self) -> RoomKeys {
        room_keys_copy(self.keys.as_ref().expect("room keys"))
    }

    fn emit(&self, event: RoomEvent) {
        let _ = self.events.send(event);
    }

    fn handle_command(&mut self, command: RoomCommand, now: i64) {
        match command {
            RoomCommand::Create => {
                let code = make_room_code(&random_bytes(9));
                self.start(&code, now);
            }
            RoomCommand::Join(text) => {
                if let ParsedRoomCode::Valid(code) = parse_room_code(&text, false) {
                    self.start(&code, now);
                }
            }
            RoomCommand::Leave => {
                if self.phase != Phase::Idle {
                    self.stop(now, false);
                    self.emit(RoomEvent::SaveCode(None));
                }
            }
            RoomCommand::Mute { peer, on } => {
                if let Some(i) = self.member_idx(&peer) {
                    self.members[i].muted = on;
                    self.emit_state(now);
                }
            }
            RoomCommand::SetInternet(on) => self.set_internet(on, now),
            RoomCommand::SetFlags { paused, room_muted } => {
                if paused == self.paused && room_muted == self.room_muted {
                    return;
                }
                self.paused = paused;
                self.room_muted = room_muted;
                self.announce(None, now);
                self.emit_state(now);
            }
            RoomCommand::SendPing { number, fx, fy, id } => {
                self.send_action(Action::Ping { id, d: number, x: fx, y: fy }, now)
            }
            RoomCommand::SendEmote { number, fx, fy, reference } => {
                self.send_action(Action::Emote { reference, d: number, x: fx, y: fy }, now)
            }
            RoomCommand::Profile(profile) => {
                self.profile = profile;
                self.announce(None, now);
                self.emit_state(now);
            }
            RoomCommand::RetryInternet => {
                if let Some(i) = self.active_slot(TransportKind::Internet) {
                    self.nets[i].net.retry();
                }
            }
            RoomCommand::Wake => {
                self.woke_at = now;
                for i in 0..self.nets.len() {
                    if self.nets[i].active {
                        self.nets[i].net.wake();
                    }
                }
            }
            RoomCommand::Quit => {
                if self.phase != Phase::Idle {
                    self.stop(now, true);
                }
                self.quit = true;
            }
        }
    }

    fn start(&mut self, code: &str, now: i64) {
        if self.phase != Phase::Idle {
            self.stop(now, false);
            self.emit(RoomEvent::SaveCode(None));
        }
        self.self_id = hex(&random_bytes(8));
        self.seq = 0;
        self.phase = Phase::Joining;
        self.code = Some(code.to_string());
        self.guard = ReplayGuard::default();
        self.alone_since = Some(now);
        self.emit_state(now);

        let keys = match &self.keys_override {
            Some(keys) => room_keys_copy(keys),
            None => derive_room_keys(code),
        };
        self.keys = Some(keys);
        self.joined_at = now;
        self.last_tick = now;
        self.settle_short = false;
        self.settle_at = Some(now + SETTLE_CAP_MS);

        // The port may have been taken when the process started: bind it again now, if it is free.
        if self.slot_idx(TransportKind::Lan).is_none() {
            let (tx, rx) = channel();
            if let Some(handle) = self.lan_factory.as_mut().and_then(|make| make(tx)) {
                self.add_net(Box::new(handle), rx);
            }
        }

        for i in 0..self.nets.len() {
            if self.nets[i].kind == TransportKind::Internet && !self.internet_allowed {
                continue;
            }
            self.nets[i].active = true;
            self.nets[i].net.start(self.room_keys());
        }
        self.announce(None, now);
        self.check_settle(now);
        self.emit_state(now);
    }

    fn stop(&mut self, now: i64, drain: bool) {
        if self.phase != Phase::Idle {
            if self.keys.is_some() {
                let seq = self.next_seq();
                let bye = self.seal(&RoomMessage::Bye { peer: self.self_id.clone(), seq, ts: now as u64 });
                for i in 0..self.nets.len() {
                    if self.nets[i].active {
                        self.spread(i, &bye);
                    }
                }
            }
            for i in 0..self.nets.len() {
                if self.nets[i].active {
                    self.nets[i].active = false;
                    self.nets[i].net.stop();
                }
            }
            self.phase = Phase::Idle;
            self.code = None;
            self.keys = None;
            self.members.clear();
            self.answered.clear();
            self.stale_strikes.clear();
            self.alone_since = None;
            self.last_presence = [None; 2];
            self.shared_at = [None; 2];
            self.settle_at = None;
            self.settle_short = false;
            self.emit_state(now);
        }
        if drain {
            for slot in std::mem::take(&mut self.nets) {
                slot.net.shutdown();
            }
        }
    }

    // Adds a long-lived transport, with a pump when there is a manager thread to feed.
    fn add_net(&mut self, net: Box<dyn Net>, receiver: Receiver<TransportEvent>) {
        let tag = self.next_tag;
        self.next_tag += 1;
        let kind = net.kind();
        let shared = net.shared();
        self.nets.push(Slot { tag, kind, shared, link: Link::Unavailable, active: false, net, ev: Some(receiver) });
        let i = self.nets.len() - 1;
        if let Some(mux) = self.mux.clone() {
            let receiver = self.nets[i].ev.take().expect("receiver");
            thread::spawn(move || {
                while let Ok(ev) = receiver.recv() {
                    if mux.send(Input::Event { tag, ev }).is_err() {
                        break;
                    }
                }
            });
        }
    }

    fn set_internet(&mut self, on: bool, now: i64) {
        if on == self.internet_allowed {
            return;
        }
        self.internet_allowed = on;
        if self.keys.is_some() {
            if on {
                if let Some(i) = self.slot_idx(TransportKind::Internet) {
                    self.nets[i].active = true;
                    self.nets[i].net.start(self.room_keys());
                }
                self.announce(Some(TransportKind::Internet), now);
            } else {
                self.shared_at[ki(TransportKind::Internet)] = None;
                if let Some(i) = self.slot_idx(TransportKind::Internet) {
                    self.nets[i].active = false;
                    self.nets[i].net.stop();
                }
                for m in &mut self.members {
                    if let Some(route) = m.paths[1].take() {
                        self.answered.remove(&route.key);
                    }
                }
            }
        }
        self.check_settle(now);
        self.emit_state(now);
    }

    fn check_settle(&mut self, now: i64) {
        if self.phase != Phase::Joining || self.settle_at.is_none() || self.settle_short {
            return;
        }
        for slot in &self.nets {
            if slot.active && slot.shared && slot.link != Link::Ok {
                return;
            }
        }
        self.settle_short = true;
        self.settle_at = Some((self.joined_at + SETTLE_CAP_MS).min(now + SETTLE_MS));
    }

    fn settled(&mut self, now: i64) {
        self.settle_at = None;
        if self.members.len() >= MAX_MEMBERS {
            self.stop(now, false);
            self.emit(RoomEvent::Toast(Toast::Full));
            return;
        }
        self.phase = Phase::Active;
        self.emit(RoomEvent::SaveCode(self.code.clone()));
        let code = format_room_code(self.code.as_ref().expect("code"));
        self.emit(RoomEvent::Toast(Toast::JoinedRoom { code }));
        self.emit_state(now);
    }

    fn fire_due(&mut self, now: i64) {
        if self.phase == Phase::Idle {
            return;
        }
        if let Some(at) = self.settle_at {
            if now >= at {
                self.settled(now);
            }
        }
        for kind in [TransportKind::Lan, TransportKind::Internet] {
            if let Some(at) = self.shared_at[ki(kind)] {
                if now >= at {
                    self.shared_at[ki(kind)] = None;
                    if let Some(i) = self.active_slot(kind) {
                        self.send_presence(i, now);
                    }
                }
            }
        }
        if now - self.last_tick >= TICK_MS {
            self.tick(now);
        }
    }

    fn next_deadline(&self, now: i64) -> Option<i64> {
        if self.phase == Phase::Idle {
            return None;
        }
        let mut next = self.last_tick + TICK_MS;
        if let Some(at) = self.settle_at {
            next = next.min(at);
        }
        for kind in [TransportKind::Lan, TransportKind::Internet] {
            if let Some(at) = self.shared_at[ki(kind)] {
                next = next.min(at);
            }
        }
        Some(next.max(now + 1))
    }

    fn tick(&mut self, now: i64) {
        if now - self.last_tick > SLEEP_GAP_MS {
            self.woke_at = now;
        }
        self.last_tick = now;
        self.stale_strikes.retain(|_, (_, at)| now - *at < SKEW_HINT_MS);
        let dropped: Vec<usize> = self
            .members
            .iter()
            .enumerate()
            .filter(|(_, m)| now - self.heard_at(m) >= DROP_MS[if m.via_internet { 1 } else { 0 }])
            .map(|(i, _)| i)
            .collect();
        for i in dropped.into_iter().rev() {
            self.remove(i, true, now);
        }
        for kind in [TransportKind::Lan, TransportKind::Internet] {
            if let Some(i) = self.active_slot(kind) {
                let due = match self.last_presence[ki(kind)] {
                    Some(last) => now - last >= PRESENCE_MS[ki(kind)],
                    None => true,
                };
                if due {
                    self.send_presence(i, now);
                }
            }
        }
        self.emit_state(now);
    }

    fn on_event(&mut self, tag: u64, event: TransportEvent, now: i64) {
        let Some(i) = self.nets.iter().position(|s| s.tag == tag) else {
            return;
        };
        let kind = self.nets[i].kind;
        match event {
            TransportEvent::Packet { peer, bytes } => self.on_packet(i, &peer, &bytes, now),
            TransportEvent::PeerGone { peer } => self.on_peer_gone(i, &peer, now),
            TransportEvent::Status(link) => {
                self.nets[i].link = link;
                if link == Link::Ok && !self.nets[i].shared {
                    self.announce(Some(kind), now);
                }
                self.check_settle(now);
                self.emit_state(now);
            }
            TransportEvent::LinkUp => self.announce(Some(kind), now),
        }
    }

    #[cfg(test)]
    fn drain_events(&mut self, now: i64) {
        for i in 0..self.nets.len() {
            loop {
                let event = self.nets[i].ev.as_ref().and_then(|rx| rx.try_recv().ok());
                let Some(event) = event else {
                    break;
                };
                let tag = self.nets[i].tag;
                self.on_event(tag, event, now);
            }
        }
    }

    fn on_packet(&mut self, i: usize, key: &str, packet: &[u8], now: i64) {
        let Some(msg_key) = self.keys.as_ref().map(|k| k.msg_key) else {
            return;
        };
        let Some(plain) = open(&msg_key, packet) else {
            return;
        };
        let Some(msg) = decode_message(&plain) else {
            return;
        };
        if msg.peer() == self.self_id {
            return;
        }
        let peer = msg.peer().to_string();
        match self.guard.accept(&peer, msg.seq(), msg.ts() as i64, now) {
            Verdict::Ok => {
                self.stale_strikes.remove(&peer);
            }
            Verdict::Stale => {
                if matches!(msg, RoomMessage::Presence { .. }) {
                    let entry = self.stale_strikes.entry(peer).or_insert((0, now));
                    entry.0 += 1;
                    entry.1 = now;
                    self.emit_state(now);
                }
                return;
            }
            _ => return,
        }
        if matches!(msg, RoomMessage::Presence { .. }) {
            return self.on_presence(i, key, msg, now);
        }
        if matches!(msg, RoomMessage::Bye { .. }) {
            self.guard.mark_gone(&peer);
            if let Some(mi) = self.member_idx(&peer) {
                self.remove(mi, false, now);
            }
            return;
        }
        let action = match msg {
            RoomMessage::Ping { ping, d, x, y, .. } => Action::Ping { id: ping, d, x, y },
            RoomMessage::Emote { e, d, x, y, .. } => Action::Emote { reference: e, d, x, y },
            _ => return,
        };
        self.on_action(i, key, &peer, action, now);
    }

    fn on_presence(&mut self, i: usize, key: &str, msg: RoomMessage, now: i64) {
        let RoomMessage::Presence { peer, proto, app, name, color, status, .. } = msg else {
            return;
        };
        let is_new = self.member_idx(&peer).is_none();
        if is_new {
            self.members.push(MemberCore {
                peer: peer.clone(),
                name: name.clone(),
                color,
                status: status.as_str().to_string(),
                needs_update: false,
                no_emotes: false,
                muted: false,
                first_seen: now,
                via_internet: false,
                paths: [None, None],
            });
            self.alone_since = None;
        }
        let mi = self.member_idx(&peer).expect("just added");
        let member = &mut self.members[mi];
        member.name = name;
        member.color = color;
        member.status = status.as_str().to_string();
        member.needs_update = proto != PROTOCOL_VERSION;
        member.no_emotes = is_older_than(&app, EMOTES_SINCE);
        self.heard(mi, i, key, now);
        if !self.answered.contains(key) {
            self.answered.insert(key.to_string());
            let kind = self.nets[i].kind;
            if self.nets[i].shared {
                self.announce(Some(kind), now);
            } else {
                let packet = self.presence_packet(now);
                self.nets[i].net.send_to(key, &packet);
            }
        }
        if is_new && self.phase == Phase::Active {
            let name = self.members[mi].name.clone();
            self.emit(RoomEvent::Toast(Toast::Joined { name }));
        }
        self.emit_state(now);
    }

    fn on_action(&mut self, i: usize, key: &str, peer: &str, action: Action, now: i64) {
        let Some(mi) = self.member_idx(peer) else {
            return;
        };
        self.heard(mi, i, key, now);
        let member = &self.members[mi];
        if self.paused || self.room_muted || member.muted || member.needs_update {
            return;
        }
        let limit = self.profile.limit as f64;
        if !self.limiter.allow(peer, limit) {
            return;
        }
        let tag = Tag { name: self.members[mi].name.clone(), color: self.members[mi].color };
        match action {
            Action::Ping { id, d, x, y } => {
                self.emit(RoomEvent::Ping { number: d, fx: x, fy: y, id, tag })
            }
            Action::Emote { reference, d, x, y } => self.emit(RoomEvent::Emote {
                number: d,
                fx: x,
                fy: y,
                reference,
                peer: peer.to_string(),
                tag,
            }),
        }
    }

    fn on_peer_gone(&mut self, i: usize, key: &str, now: i64) {
        let kind = ki(self.nets[i].kind);
        for m in &mut self.members {
            if m.paths[kind].as_ref().is_some_and(|p| p.key == key) {
                m.paths[kind] = None;
            }
        }
        self.answered.remove(key);
        self.emit_state(now);
    }

    fn heard(&mut self, mi: usize, i: usize, key: &str, now: i64) {
        let kind = ki(self.nets[i].kind);
        let member = &mut self.members[mi];
        member.paths[kind] = Some(Route { key: key.to_string(), seen_at: now });
        if kind == ki(TransportKind::Internet) {
            member.via_internet = true;
        }
    }

    fn heard_at(&self, m: &MemberCore) -> i64 {
        m.paths
            .iter()
            .flatten()
            .map(|p| p.seen_at)
            .chain([m.first_seen, self.woke_at])
            .max()
            .unwrap_or(m.first_seen)
    }

    fn remove(&mut self, mi: usize, disconnected: bool, now: i64) {
        let member = self.members.remove(mi);
        self.limiter.forget(&member.peer);
        for route in member.paths.iter().flatten() {
            self.answered.remove(&route.key);
        }
        if self.members.is_empty() && self.phase != Phase::Idle {
            self.alone_since = Some(now);
        }
        let name = member.name.clone();
        let toast = if disconnected { Toast::Lost { name } } else { Toast::Left { name } };
        self.emit(RoomEvent::Toast(toast));
        self.emit_state(now);
    }

    fn send_action(&mut self, action: Action, now: i64) {
        if self.phase == Phase::Idle || self.keys.is_none() || self.paused {
            return;
        }
        let seq = self.next_seq();
        let packet = match &action {
            Action::Ping { id, d, x, y } => self.seal(&RoomMessage::Ping {
                peer: self.self_id.clone(),
                seq,
                ts: now as u64,
                ping: id.clone(),
                d: *d,
                x: *x,
                y: *y,
            }),
            Action::Emote { reference, d, x, y } => self.seal(&RoomMessage::Emote {
                peer: self.self_id.clone(),
                seq,
                ts: now as u64,
                e: reference.clone(),
                d: *d,
                x: *x,
                y: *y,
            }),
        };
        let mut shared: Vec<usize> = Vec::new();
        for mi in 0..self.members.len() {
            if let Some((i, key)) = self.route(mi, now) {
                if self.nets[i].shared {
                    if !shared.contains(&i) {
                        shared.push(i);
                    }
                } else {
                    self.nets[i].net.send_to(&key, &packet);
                }
            }
        }
        for i in shared {
            let kind = self.nets[i].kind;
            if self.relay_cap.allow_burst(kind_key(kind), RELAY_PING_RATE, RELAY_PING_BURST) {
                self.nets[i].net.broadcast(&packet);
            }
        }
    }

    // LAN while it is alive, else internet, else nothing.
    fn route(&self, mi: usize, now: i64) -> Option<(usize, String)> {
        for kind in [TransportKind::Lan, TransportKind::Internet] {
            let Some(route) = self.members[mi].paths[ki(kind)].as_ref() else {
                continue;
            };
            let Some(i) = self.active_slot(kind) else {
                continue;
            };
            if now - route.seen_at < ALIVE_MS[ki(kind)] {
                return Some((i, route.key.clone()));
            }
        }
        None
    }

    fn path_of(&self, mi: usize, now: i64) -> Path {
        if let Some((i, _)) = self.route(mi, now) {
            return match self.nets[i].kind {
                TransportKind::Lan => Path::Lan,
                TransportKind::Internet => Path::Internet,
            };
        }
        if now - self.members[mi].first_seen < UNREACHABLE_MS {
            Path::Connecting
        } else {
            Path::Unreachable
        }
    }

    fn status(&self) -> Status {
        if self.paused {
            Status::Paused
        } else if self.room_muted {
            Status::Muted
        } else {
            Status::On
        }
    }

    fn presence_packet(&mut self, now: i64) -> Vec<u8> {
        let msg = RoomMessage::Presence {
            peer: self.self_id.clone(),
            seq: self.next_seq(),
            ts: now as u64,
            proto: PROTOCOL_VERSION,
            app: self.app_version.clone(),
            name: self.profile.name.clone(),
            color: self.profile.color,
            status: self.status(),
        };
        self.seal(&msg)
    }

    fn seal(&self, msg: &RoomMessage) -> Vec<u8> {
        let keys = self.keys.as_ref().expect("room keys");
        seal(&keys.msg_key, &encode_message(msg))
    }

    fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn announce(&mut self, only: Option<TransportKind>, now: i64) {
        if self.keys.is_none() {
            return;
        }
        let targets: Vec<usize> = (0..self.nets.len())
            .filter(|&i| self.nets[i].active && only.is_none_or(|kind| self.nets[i].kind == kind))
            .collect();
        for i in targets {
            if !self.nets[i].shared {
                self.send_presence(i, now);
                continue;
            }
            let kind = ki(self.nets[i].kind);
            let Some(last) = self.last_presence[kind] else {
                self.send_presence(i, now);
                continue;
            };
            let wait = last + SHARED_PRESENCE_MS - now;
            if wait <= 0 {
                self.send_presence(i, now);
            } else if self.shared_at[kind].is_none() {
                self.shared_at[kind] = Some(now + wait);
            }
        }
    }

    fn send_presence(&mut self, i: usize, now: i64) {
        self.shared_at[ki(self.nets[i].kind)] = None;
        let packet = self.presence_packet(now);
        self.spread(i, &packet);
        self.last_presence[ki(self.nets[i].kind)] = Some(now);
    }

    // Broadcast, and on LAN also straight to every member heard there: Wi-Fi broadcast is lossy.
    fn spread(&mut self, i: usize, packet: &[u8]) {
        let kind = ki(self.nets[i].kind);
        self.nets[i].net.broadcast(packet);
        if self.nets[i].shared {
            return;
        }
        let keys: Vec<String> = self
            .members
            .iter()
            .filter_map(|m| m.paths[kind].as_ref().map(|p| p.key.clone()))
            .collect();
        for key in keys {
            self.nets[i].net.send_to(&key, packet);
        }
    }

    fn member_idx(&self, peer: &str) -> Option<usize> {
        self.members.iter().position(|m| m.peer == peer)
    }

    fn slot_idx(&self, kind: TransportKind) -> Option<usize> {
        self.nets.iter().position(|s| s.kind == kind)
    }

    fn active_slot(&self, kind: TransportKind) -> Option<usize> {
        self.nets.iter().position(|s| s.kind == kind && s.active)
    }

    fn link_of(&self, kind: TransportKind) -> Link {
        if self.phase == Phase::Idle {
            return Link::Unavailable;
        }
        self.active_slot(kind).map(|i| self.nets[i].link).unwrap_or(Link::Unavailable)
    }

    fn build_state(&self, now: i64) -> RoomState {
        let members = if self.phase == Phase::Idle {
            Vec::new()
        } else {
            let mut members = vec![Member {
                peer: self.self_id.clone(),
                name: self.profile.name.clone(),
                color: self.profile.color,
                status: self.status().as_str().to_string(),
                path: Path::Lan,
                needs_update: false,
                no_emotes: false,
                muted: false,
                me: true,
            }];
            members.extend(self.members.iter().enumerate().map(|(i, m)| Member {
                peer: m.peer.clone(),
                name: m.name.clone(),
                color: m.color,
                status: m.status.clone(),
                path: self.path_of(i, now),
                needs_update: m.needs_update,
                no_emotes: m.no_emotes,
                muted: m.muted,
                me: false,
            }));
            members
        };
        RoomState {
            phase: self.phase,
            code: self.code.as_ref().map(|c| format_room_code(c)),
            members,
            lan: self.link_of(TransportKind::Lan),
            internet: self.link_of(TransportKind::Internet),
            internet_allowed: self.internet_allowed,
            paused: self.paused,
            room_muted: self.room_muted,
            lonely: self.alone_since.is_some_and(|since| now - since >= LONELY_MS),
            clock_skew: self
                .stale_strikes
                .values()
                .any(|(count, at)| *count >= STALE_STRIKES && now - *at < SKEW_HINT_MS),
        }
    }

    fn emit_state(&mut self, now: i64) {
        let state = Arc::new(self.build_state(now));
        if self.last_state.as_ref() == Some(&state) {
            return;
        }
        self.last_state = Some(state.clone());
        *self.cell.lock() = state.clone();
        self.emit(RoomEvent::State(state));
    }
}

fn run(mut core: Core, rx: Receiver<Input>) {
    loop {
        let now = now_ms();
        core.fire_due(now);
        if core.quit {
            break;
        }
        let wait = core
            .next_deadline(now)
            .map(|d| (d - now).clamp(1, 60_000) as u64)
            .unwrap_or(60_000);
        match rx.recv_timeout(Duration::from_millis(wait)) {
            Ok(Input::Command(command)) => core.handle_command(command, now_ms()),
            Ok(Input::Event { tag, ev }) => core.on_event(tag, ev, now_ms()),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    core.stop(now_ms(), true);
}

pub struct Room {
    state: Arc<Mutex<Arc<RoomState>>>,
    tx: Sender<Input>,
    join: Option<JoinHandle<()>>,
}

impl Room {
    pub fn start(
        profile: Profile,
        app_version: String,
        allow_internet: bool,
        relay_urls: Vec<String>,
        events: Sender<RoomEvent>,
    ) -> Room {
        let (tx, rx) = channel();
        let state = Arc::new(Mutex::new(Arc::new(RoomState {
            phase: Phase::Idle,
            code: None,
            members: Vec::new(),
            lan: Link::Unavailable,
            internet: Link::Unavailable,
            internet_allowed: allow_internet,
            paused: false,
            room_muted: false,
            lonely: false,
            clock_skew: false,
        })));
        let mut nets: Vec<(Box<dyn Net>, Receiver<TransportEvent>)> = Vec::new();
        let (lan_events, lan_inbox) = channel();
        if let Some(handle) = super::lan::start(lan_events) {
            nets.push((Box::new(handle) as Box<dyn Net>, lan_inbox));
        }
        if !relay_urls.is_empty() {
            let (relay_events, relay_inbox) = channel();
            nets.push((Box::new(super::relay::spawn(relay_urls, relay_events)) as Box<dyn Net>, relay_inbox));
        }
        let lan_factory: LanFactory = Box::new(super::lan::start);
        let core = Core::new(
            events,
            state.clone(),
            nets,
            Some(tx.clone()),
            Some(lan_factory),
            None,
            Arc::new(|| now_ms() as u64),
            profile,
            app_version,
            allow_internet,
        );
        let join = thread::spawn(move || run(core, rx));
        Room { state, tx, join: Some(join) }
    }

    pub fn send(&self, command: RoomCommand) {
        let _ = self.tx.send(Input::Command(command));
    }

    pub fn state(&self) -> Arc<RoomState> {
        self.state.lock().clone()
    }

    pub fn shutdown(mut self) {
        let _ = self.tx.send(Input::Command(RoomCommand::Quit));
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::transport::TransportCommand;
    use std::sync::atomic::{AtomicI64, Ordering};

    const KEY: [u8; 32] = [1u8; 32];
    const T0: i64 = 1_780_000_000_000;

    type Shared = Arc<Mutex<FakeState>>;

    struct FakeState {
        kind: TransportKind,
        shared: bool,
        start_link: Link,
        ev_tx: Option<Sender<TransportEvent>>,
        cmds: Option<Receiver<TransportCommand>>,
        started: Option<RoomKeys>,
        starts: u32,
        stops: u32,
        shutdowns: u32,
        wakes: u32,
        retries: u32,
        sent: Vec<(String, Vec<u8>)>,
    }

    impl FakeState {
        fn new(kind: TransportKind, shared: bool) -> FakeState {
            FakeState {
                kind,
                shared,
                start_link: Link::Ok,
                ev_tx: None,
                cmds: None,
                started: None,
                starts: 0,
                stops: 0,
                shutdowns: 0,
                wakes: 0,
                retries: 0,
                sent: Vec::new(),
            }
        }
    }

    struct FakeNet {
        state: Shared,
        tx: Sender<TransportCommand>,
    }

    impl FakeNet {
        fn emit(&self, event: TransportEvent) {
            if let Some(tx) = self.state.lock().ev_tx.clone() {
                let _ = tx.send(event);
            }
        }
    }

    impl Net for FakeNet {
        fn kind(&self) -> TransportKind {
            self.state.lock().kind
        }
        fn shared(&self) -> bool {
            self.state.lock().shared
        }
        fn start(&self, keys: RoomKeys) {
            let link = self.state.lock().start_link;
            self.emit(TransportEvent::Status(link));
            let _ = self.tx.send(TransportCommand::Start { keys });
        }
        fn send_to(&self, peer: &str, bytes: &[u8]) {
            let _ = self.tx.send(TransportCommand::SendTo { peer: peer.to_string(), bytes: bytes.to_vec() });
        }
        fn broadcast(&self, bytes: &[u8]) {
            let _ = self.tx.send(TransportCommand::Broadcast { bytes: bytes.to_vec() });
        }
        fn stop(&self) {
            self.emit(TransportEvent::Status(Link::Unavailable));
            let _ = self.tx.send(TransportCommand::Stop);
        }
        fn wake(&self) {
            let _ = self.tx.send(TransportCommand::Wake);
        }
        fn retry(&self) {
            let _ = self.tx.send(TransportCommand::Retry);
        }
        fn shutdown(self: Box<Self>) {
            self.emit(TransportEvent::Status(Link::Unavailable));
            let _ = self.tx.send(TransportCommand::Shutdown);
        }
    }

    #[derive(Debug, PartialEq)]
    struct GotPing {
        number: u32,
        fx: f64,
        fy: f64,
        id: String,
        tag: (String, u32),
    }

    #[derive(Debug, PartialEq)]
    struct GotEmote {
        number: u32,
        fx: f64,
        fy: f64,
        reference: String,
        peer: String,
        tag: (String, u32),
    }

    struct Harness {
        core: Core,
        clock: Arc<AtomicI64>,
        lan: Shared,
        net: Shared,
        events: Receiver<RoomEvent>,
        pings: Vec<GotPing>,
        emotes: Vec<GotEmote>,
        toasts: Vec<Toast>,
        saved: Vec<Option<String>>,
    }

    impl Harness {
        fn new(allow_internet: bool) -> Harness {
            let (events_tx, events) = channel();
            let clock = Arc::new(AtomicI64::new(T0));
            let lan = Arc::new(Mutex::new(FakeState::new(TransportKind::Lan, false)));
            let net = Arc::new(Mutex::new(FakeState::new(TransportKind::Internet, true)));
            let mut nets: Vec<(Box<dyn Net>, Receiver<TransportEvent>)> = Vec::new();
            for state in [lan.clone(), net.clone()] {
                let (ev_tx, ev_rx) = channel();
                let (cmd_tx, cmd_rx) = channel();
                {
                    let mut s = state.lock();
                    s.ev_tx = Some(ev_tx);
                    s.cmds = Some(cmd_rx);
                }
                nets.push((Box::new(FakeNet { state, tx: cmd_tx }) as Box<dyn Net>, ev_rx));
            }
            let cell = Arc::new(Mutex::new(Arc::new(RoomState {
                phase: Phase::Idle,
                code: None,
                members: Vec::new(),
                lan: Link::Unavailable,
                internet: Link::Unavailable,
                internet_allowed: allow_internet,
                paused: false,
                room_muted: false,
                lonely: false,
                clock_skew: false,
            })));
            let core = Core::new(
                events_tx,
                cell,
                nets,
                None,
                None,
                Some(RoomKeys { msg_key: KEY, topic: "room".to_string() }),
                Arc::new({
                    let clock = clock.clone();
                    move || clock.load(Ordering::Relaxed) as u64
                }),
                Profile { name: "Me".to_string(), color: 1, limit: 5 },
                "0.3.0".to_string(),
                allow_internet,
            );
            Harness { core, clock, lan, net, events, pings: Vec::new(), emotes: Vec::new(), toasts: Vec::new(), saved: Vec::new() }
        }

        fn now(&self) -> i64 {
            self.clock.load(Ordering::Relaxed)
        }

        fn sync(&mut self) {
            let now = self.now();
            self.core.drain_events(now);
            for state in [&self.lan, &self.net] {
                let commands: Vec<TransportCommand> = {
                    let s = state.lock();
                    match &s.cmds {
                        Some(rx) => std::iter::from_fn(|| rx.try_recv().ok()).collect(),
                        None => Vec::new(),
                    }
                };
                let mut s = state.lock();
                for command in commands {
                    match command {
                        TransportCommand::Start { keys } => {
                            s.starts += 1;
                            s.started = Some(keys);
                        }
                        TransportCommand::Stop => {
                            s.stops += 1;
                            s.started = None;
                        }
                        TransportCommand::Shutdown => {
                            s.shutdowns += 1;
                            s.started = None;
                        }
                        TransportCommand::Wake => s.wakes += 1,
                        TransportCommand::Retry => s.retries += 1,
                        TransportCommand::SendTo { peer, bytes } => s.sent.push((peer, bytes)),
                        TransportCommand::Broadcast { bytes } => s.sent.push(("*".to_string(), bytes)),
                    }
                }
            }
            while let Ok(event) = self.events.try_recv() {
                match event {
                    RoomEvent::Ping { number, fx, fy, id, tag } => {
                        self.pings.push(GotPing { number, fx, fy, id, tag: (tag.name, tag.color) })
                    }
                    RoomEvent::Emote { number, fx, fy, reference, peer, tag } => self.emotes.push(GotEmote {
                        number,
                        fx,
                        fy,
                        reference,
                        peer,
                        tag: (tag.name, tag.color),
                    }),
                    RoomEvent::Toast(toast) => self.toasts.push(toast),
                    RoomEvent::SaveCode(code) => self.saved.push(code),
                    RoomEvent::State(_) => {}
                }
            }
        }

        fn cmd(&mut self, command: RoomCommand) {
            let now = self.now();
            self.core.handle_command(command, now);
            self.sync();
        }

        fn set_now(&self, now: i64) {
            self.clock.store(now, Ordering::Relaxed);
        }

        fn advance(&mut self, ms: i64) {
            let target = self.now() + ms;
            loop {
                let now = self.now();
                self.core.fire_due(now);
                let Some(next) = self.core.next_deadline(now) else {
                    break;
                };
                if next > target || next <= now {
                    break;
                }
                self.set_now(next);
            }
            self.set_now(target);
            let now = self.now();
            self.core.fire_due(now);
            self.sync();
        }

        // The clock moves with no timers running: the computer slept.
        fn jump(&mut self, ms: i64) {
            let now = self.now() + ms;
            self.set_now(now);
        }

        fn state(&self) -> Arc<RoomState> {
            self.core.state()
        }

        fn create(&mut self) {
            self.cmd(RoomCommand::Create);
        }

        fn join(&mut self, text: &str) {
            self.cmd(RoomCommand::Join(text.to_string()));
        }

        fn join_active(&mut self, code: &str) {
            self.join(&format_room_code(code));
            self.advance(3000);
        }

        fn deliver(&mut self, kind: TransportKind, peer: &str, bytes: Vec<u8>) {
            let state = self.fake(kind);
            let tx = state.lock().ev_tx.clone();
            if let Some(tx) = tx {
                let _ = tx.send(TransportEvent::Packet { peer: peer.to_string(), bytes });
            }
            self.sync();
        }

        fn set_link(&mut self, kind: TransportKind, link: Link) {
            let state = self.fake(kind);
            let tx = state.lock().ev_tx.clone();
            if let Some(tx) = tx {
                let _ = tx.send(TransportEvent::Status(link));
            }
            self.sync();
        }

        fn link_up(&mut self, kind: TransportKind) {
            let state = self.fake(kind);
            let tx = state.lock().ev_tx.clone();
            if let Some(tx) = tx {
                let _ = tx.send(TransportEvent::LinkUp);
            }
            self.sync();
        }

        fn fake(&self, kind: TransportKind) -> &Shared {
            match kind {
                TransportKind::Lan => &self.lan,
                TransportKind::Internet => &self.net,
            }
        }

        fn messages(&self, kind: TransportKind, to: Option<&str>) -> Vec<RoomMessage> {
            let s = self.fake(kind).lock();
            s.sent
                .iter()
                .filter(|(peer, _)| to.is_none_or(|t| peer == t))
                .filter_map(|(_, bytes)| open(&KEY, bytes).and_then(|plain| decode_message(&plain)))
                .collect()
        }

        fn sent_to(&self, kind: TransportKind, to: &str) -> Vec<RoomMessage> {
            self.messages(kind, Some(to))
        }

        fn all_sent(&self, kind: TransportKind) -> Vec<RoomMessage> {
            self.messages(kind, None)
        }

        fn clear(&self, kind: TransportKind) {
            self.fake(kind).lock().sent.clear();
        }

        fn me(&self) -> String {
            self.all_sent(TransportKind::Lan)[0].peer().to_string()
        }

        fn set_limit(&mut self, limit: u32) {
            self.core.profile.limit = limit;
        }
    }

    fn code() -> String {
        make_room_code(&[1, 2, 3, 4, 5, 6, 7, 8, 9])
    }

    fn presence_msg(peer: &str, seq: u64, ts: i64, name: &str, color: u32, proto: u32, app: &str) -> RoomMessage {
        RoomMessage::Presence {
            peer: peer.to_string(),
            seq,
            ts: ts as u64,
            proto,
            app: app.to_string(),
            name: name.to_string(),
            color,
            status: Status::On,
        }
    }

    struct Remote {
        peer: String,
        key: String,
        kind: TransportKind,
        seq: u64,
    }

    impl Remote {
        fn new(peer: &str, key: &str, kind: TransportKind) -> Remote {
            Remote { peer: peer.to_string(), key: key.to_string(), kind, seq: 0 }
        }

        fn next(&mut self) -> u64 {
            self.seq += 1;
            self.seq
        }

        fn send(&mut self, h: &mut Harness, msg: RoomMessage) {
            let kind = self.kind;
            let key = self.key.clone();
            self.send_on(h, kind, &key, msg);
        }

        fn send_on(&mut self, h: &mut Harness, kind: TransportKind, key: &str, msg: RoomMessage) {
            let bytes = seal(&KEY, &encode_message(&msg));
            h.deliver(kind, key, bytes);
        }

        fn presence_on(&mut self, h: &mut Harness, kind: TransportKind, key: &str, name: &str, color: u32) {
            let (peer, seq, ts) = (self.peer.clone(), self.next(), h.now());
            self.send_on(h, kind, key, presence_msg(&peer, seq, ts, name, color, PROTOCOL_VERSION, "0.3.0"));
        }

        fn presence(&mut self, h: &mut Harness, name: &str, color: u32) {
            self.presence_full(h, name, color, PROTOCOL_VERSION, "0.3.0");
        }

        fn presence_full(&mut self, h: &mut Harness, name: &str, color: u32, proto: u32, app: &str) {
            let (peer, seq, ts) = (self.peer.clone(), self.next(), h.now());
            self.send(h, presence_msg(&peer, seq, ts, name, color, proto, app));
        }

        fn stale_presence(&mut self, h: &mut Harness, name: &str) {
            let (peer, seq, ts) = (self.peer.clone(), self.next(), h.now() - 3_600_000);
            self.send(h, presence_msg(&peer, seq, ts, name, 0, PROTOCOL_VERSION, "1"));
        }

        fn ping_at(&mut self, h: &mut Harness, id: &str, d: u32, x: f64, y: f64) {
            let (peer, seq, ts) = (self.peer.clone(), self.next(), h.now() as u64);
            self.send(h, RoomMessage::Ping { peer, seq, ts, ping: id.to_string(), d, x, y });
        }

        fn ping(&mut self, h: &mut Harness) {
            self.ping_at(h, "danger", 1, 0.5, 0.25);
        }

        fn emote_at(&mut self, h: &mut Harness, e: &str, d: u32, x: f64, y: f64) {
            let (peer, seq, ts) = (self.peer.clone(), self.next(), h.now() as u64);
            self.send(h, RoomMessage::Emote { peer, seq, ts, e: e.to_string(), d, x, y });
        }

        fn emote(&mut self, h: &mut Harness, e: &str) {
            self.emote_at(h, e, 1, 0.5, 0.25);
        }

        fn bye(&mut self, h: &mut Harness) {
            let (peer, seq, ts) = (self.peer.clone(), self.next(), h.now() as u64);
            self.send(h, RoomMessage::Bye { peer, seq, ts });
        }
    }

    fn lan_remote(peer: &str, key: &str) -> Remote {
        Remote::new(peer, key, TransportKind::Lan)
    }

    fn net_remote(peer: &str, key: &str) -> Remote {
        Remote::new(peer, key, TransportKind::Internet)
    }

    #[test]
    fn create_joins_a_new_room_announces_itself_and_becomes_active_after_3s() {
        let mut h = Harness::new(true);
        h.create();
        assert!(h.state().code.as_deref().is_some_and(|code| code.starts_with("PING-")));
        assert_eq!(h.state().phase, Phase::Joining);
        assert!(h.saved.is_empty());
        assert_eq!(h.state().members.len(), 1);
        assert_eq!(h.lan.lock().started.as_ref().unwrap().msg_key, KEY);
        let first = h.all_sent(TransportKind::Lan);
        assert!(matches!(first[0], RoomMessage::Presence { proto, name: _, color: 1, .. } if proto == PROTOCOL_VERSION));
        assert_eq!(h.state().lan, Link::Ok);

        h.advance(3000);
        let state = h.state();
        assert!(state.code.as_ref().unwrap().starts_with("PING-"));
        assert_eq!(state.phase, Phase::Active);
        assert_eq!(h.saved.len(), 1);
        assert_eq!(h.toasts, vec![Toast::JoinedRoom { code: state.code.clone().unwrap() }]);
    }

    #[test]
    fn rejects_text_without_a_code_and_a_mistyped_code() {
        let mut h = Harness::new(true);
        h.join("hello");
        let typo = format!("{}{}", &code()[..9], if &code()[9..] == "0" { "1" } else { "0" });
        h.join(&typo);
        assert!(h.lan.lock().started.is_none());
        assert_eq!(h.state().members.len(), 0);
        assert_eq!(h.lan.lock().starts, 0);
    }

    #[test]
    fn leaves_a_room_that_already_has_8_other_members() {
        let mut h = Harness::new(true);
        h.join(&code());
        for i in 0..8u32 {
            lan_remote(&format!("{:x}", i).repeat(16), &format!("lan:10.0.0.{i}:47474")).presence(&mut h, "P", 2);
        }
        h.advance(3000);
        assert_eq!(h.state().members.len(), 0);
        assert_eq!(h.toasts, vec![Toast::Full]);
        assert!(matches!(h.all_sent(TransportKind::Lan).last().unwrap(), RoomMessage::Bye { .. }));
        assert!(h.lan.lock().started.is_none());
        assert!(h.saved.is_empty());
    }

    #[test]
    fn joining_another_room_leaves_the_first_one() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let other = make_room_code(&[9, 9, 9, 9, 9, 9, 9, 9, 9]);
        h.join(&other);
        assert!(h.all_sent(TransportKind::Lan).iter().any(|m| matches!(m, RoomMessage::Bye { .. })));
        assert_eq!(h.state().code, Some(format_room_code(&other)));
    }

    #[test]
    fn adds_a_member_on_presence_and_answers_it_once_directly() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:10.0.0.2:47474");
        alex.presence(&mut h, "Alex", 4);
        alex.presence(&mut h, "Alex", 4);
        let answers: Vec<RoomMessage> = h
            .sent_to(TransportKind::Lan, "lan:10.0.0.2:47474")
            .into_iter()
            .filter(|m| matches!(m, RoomMessage::Presence { .. }))
            .collect();
        assert_eq!(answers.len(), 1);
        let state = h.state();
        assert!(state.members[0].me);
        assert_eq!(state.members[0].name, "Me");
        assert_eq!(state.members[1].peer, "1111111111111111");
        assert_eq!(state.members[1].name, "Alex");
        assert_eq!(state.members[1].color, 4);
        assert_eq!(state.members[1].path, Path::Lan);
        assert_eq!(state.members[1].status, "on");
        assert_eq!(h.toasts.last(), Some(&Toast::Joined { name: "Alex".to_string() }));
    }

    #[test]
    fn does_not_toast_members_that_were_already_there_when_joining() {
        let mut h = Harness::new(true);
        h.join(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "Alex", 2);
        h.advance(3000);
        assert_eq!(h.toasts, vec![Toast::JoinedRoom { code: h.state().code.clone().unwrap() }]);
    }

    #[test]
    fn ignores_packets_sealed_with_another_key_and_its_own_messages() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let msg = presence_msg("1111111111111111", 1, h.now(), "X", 0, PROTOCOL_VERSION, "1");
        h.deliver(TransportKind::Lan, "lan:a", seal(&[2u8; 32], &encode_message(&msg)));
        let me = h.me();
        lan_remote(&me, "lan:me").presence(&mut h, "Me", 1);
        assert_eq!(h.state().members.len(), 1);
    }

    #[test]
    fn drops_a_member_after_15s_of_lan_silence() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "Alex", 2);
        h.advance(6000);
        assert_eq!(h.state().members.len(), 2);
        h.advance(9000);
        assert_eq!(h.state().members.len(), 1);
        assert_eq!(h.toasts.last(), Some(&Toast::Lost { name: "Alex".to_string() }));
    }

    #[test]
    fn gives_members_a_fresh_grace_period_after_the_computer_slept() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "Alex", 2);
        h.jump(60_000);
        h.advance(1000);
        assert_eq!(h.state().members.len(), 2);
        assert!(!h.toasts.iter().any(|t| matches!(t, Toast::Lost { .. })));
        h.advance(15_000);
        assert_eq!(h.state().members.len(), 1);
        assert_eq!(h.toasts.last(), Some(&Toast::Lost { name: "Alex".to_string() }));
    }

    #[test]
    fn removes_a_member_that_says_bye_and_ignores_it_afterwards() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Alex", 2);
        alex.bye(&mut h);
        assert_eq!(h.state().members.len(), 1);
        assert_eq!(h.toasts.last(), Some(&Toast::Left { name: "Alex".to_string() }));
        alex.presence(&mut h, "Alex", 2);
        assert_eq!(h.state().members.len(), 1);
    }

    #[test]
    fn never_adds_answers_or_counts_a_member_from_stale_packets() {
        let mut h = Harness::new(true);
        h.join(&code());
        let mut ghosts: Vec<Remote> = (0..8u32)
            .map(|i| lan_remote(&format!("{:x}", i).repeat(16), &format!("lan:66.6.6.6:{i}")))
            .collect();
        h.clear(TransportKind::Lan);
        for ghost in &mut ghosts {
            for n in 0..4 {
                ghost.stale_presence(&mut h, &format!("Old{n}"));
            }
        }
        h.advance(3000);
        assert_eq!(h.state().members.len(), 1);
        assert!(!h.lan.lock().sent.iter().any(|(to, _)| to.starts_with("lan:66.6.6.6")));
        assert_eq!(h.toasts, vec![Toast::JoinedRoom { code: h.state().code.clone().unwrap() }]);
    }

    #[test]
    fn maps_a_ping_onto_the_target_display_with_the_senders_tag() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Alex", 4);
        alex.ping_at(&mut h, "push", 1, 0.5, 0.25);
        assert_eq!(
            h.pings,
            vec![GotPing { number: 1, fx: 0.5, fy: 0.25, id: "push".to_string(), tag: ("Alex".to_string(), 4) }]
        );
    }

    #[test]
    fn shows_a_packet_that_arrives_twice_only_once() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Alex", 2);
        let seq = alex.next();
        let msg = RoomMessage::Ping { peer: alex.peer.clone(), seq, ts: h.now() as u64, ping: "danger".to_string(), d: 1, x: 0.0, y: 0.0 };
        let bytes = seal(&KEY, &encode_message(&msg));
        h.deliver(TransportKind::Lan, "lan:a", bytes.clone());
        h.deliver(TransportKind::Lan, "lan:b", bytes);
        assert_eq!(h.pings.len(), 1);
    }

    #[test]
    fn drops_pings_from_strangers_while_paused_while_muted_and_from_muted_members() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Alex", 4);
        lan_remote("2222222222222222", "lan:c").ping(&mut h);
        h.cmd(RoomCommand::SetFlags { paused: true, room_muted: false });
        alex.ping(&mut h);
        h.cmd(RoomCommand::SetFlags { paused: false, room_muted: true });
        alex.ping(&mut h);
        h.cmd(RoomCommand::SetFlags { paused: false, room_muted: false });
        h.cmd(RoomCommand::Mute { peer: alex.peer.clone(), on: true });
        alex.ping(&mut h);
        assert_eq!(h.pings.len(), 0);
        h.cmd(RoomCommand::Mute { peer: alex.peer.clone(), on: false });
        alex.ping(&mut h);
        assert_eq!(h.pings.len(), 1);
    }

    #[test]
    fn drops_pings_from_a_member_that_needs_an_update() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence_full(&mut h, "Alex", 2, 2, "0.3.0");
        assert!(h.state().members[1].needs_update);
        alex.ping(&mut h);
        assert_eq!(h.pings.len(), 0);
    }

    #[test]
    fn flags_members_too_old_for_emotes_and_no_one_else() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut old = lan_remote("1111111111111111", "lan:a");
        let mut current = lan_remote("2222222222222222", "lan:b");
        let mut odd = lan_remote("3333333333333333", "lan:c");
        old.presence_full(&mut h, "Old", 2, PROTOCOL_VERSION, "0.4.0");
        current.presence_full(&mut h, "Current", 2, PROTOCOL_VERSION, "0.5.0");
        odd.presence_full(&mut h, "Odd", 2, PROTOCOL_VERSION, "room-peer");
        let flags: Vec<(String, bool)> = h.state().members.iter().map(|m| (m.peer.clone(), m.no_emotes)).collect();
        let me = h.me();
        assert_eq!(
            flags,
            vec![
                (me, false),
                ("1111111111111111".to_string(), true),
                ("2222222222222222".to_string(), false),
                ("3333333333333333".to_string(), false),
            ]
        );
        old.presence_full(&mut h, "Old", 2, PROTOCOL_VERSION, "0.5.1");
        assert!(!h.state().members[1].no_emotes);
    }

    #[test]
    fn rate_limits_each_member_with_0_meaning_unlimited() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Alex", 2);
        h.set_limit(2);
        for _ in 0..5 {
            alex.ping(&mut h);
        }
        assert_eq!(h.pings.len(), 2);
        h.set_limit(0);
        for _ in 0..50 {
            alex.ping(&mut h);
        }
        assert_eq!(h.pings.len(), 52);
    }

    #[test]
    fn emits_remote_emotes_with_the_senders_tag() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Pa", 2);
        alex.emote_at(&mut h, "gg", 1, 0.5, 0.25);
        assert_eq!(
            h.emotes,
            vec![GotEmote {
                number: 1,
                fx: 0.5,
                fy: 0.25,
                reference: "gg".to_string(),
                peer: "1111111111111111".to_string(),
                tag: ("Pa".to_string(), 2),
            }]
        );
        assert!(h.pings.is_empty());
    }

    #[test]
    fn passes_a_custom_ref_and_an_unknown_slug_on_unchanged() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Pa", 2);
        let custom = format!("c:{}", "ab".repeat(16));
        alex.emote(&mut h, &custom);
        alex.emote(&mut h, "from_the_future");
        assert_eq!(h.emotes.iter().map(|e| e.reference.clone()).collect::<Vec<_>>(), vec![custom, "from_the_future".to_string()]);
    }

    #[test]
    fn drops_an_emote_outside_the_display_before_it_reaches_the_overlay() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Pa", 2);
        alex.emote_at(&mut h, "gg", 1, 1.5, 0.5);
        alex.emote_at(&mut h, "gg", 1, 0.5, -0.1);
        alex.emote_at(&mut h, "gg", 0, 0.5, 0.5);
        alex.emote_at(&mut h, "No Way", 1, 0.5, 0.5);
        assert!(h.emotes.is_empty());
    }

    #[test]
    fn emotes_use_the_ping_gates() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Pa", 2);
        lan_remote("2222222222222222", "lan:c").emote(&mut h, "gg");
        h.cmd(RoomCommand::SetFlags { paused: true, room_muted: false });
        alex.emote(&mut h, "gg");
        h.cmd(RoomCommand::SetFlags { paused: false, room_muted: true });
        alex.emote(&mut h, "gg");
        h.cmd(RoomCommand::SetFlags { paused: false, room_muted: false });
        h.cmd(RoomCommand::Mute { peer: alex.peer.clone(), on: true });
        alex.emote(&mut h, "gg");
        h.cmd(RoomCommand::Mute { peer: alex.peer.clone(), on: false });
        alex.presence_full(&mut h, "Pa", 2, 2, "0.3.0");
        alex.emote(&mut h, "gg");
        assert!(h.emotes.is_empty());
        alex.presence_full(&mut h, "Pa", 2, PROTOCOL_VERSION, "0.3.0");
        alex.emote(&mut h, "gg");
        assert_eq!(h.emotes.len(), 1);
    }

    #[test]
    fn shares_the_incoming_limit_with_pings() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Pa", 2);
        h.set_limit(5);
        for _ in 0..3 {
            alex.ping(&mut h);
            alex.emote(&mut h, "gg");
        }
        assert_eq!(h.pings.len() + h.emotes.len(), 5);
    }

    #[test]
    fn sends_an_emote_to_each_member_over_its_path_and_nothing_when_idle_or_paused() {
        let mut h = Harness::new(true);
        h.cmd(RoomCommand::SendEmote { number: 1, fx: 0.1, fy: 0.2, reference: "gg".to_string() });
        assert!(h.lan.lock().sent.is_empty());
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "A", 1);
        lan_remote("2222222222222222", "lan:b").presence(&mut h, "B", 1);
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::SendEmote { number: 1, fx: 0.1, fy: 0.2, reference: "gg".to_string() });
        let mut to: Vec<String> = h.lan.lock().sent.iter().map(|(p, _)| p.clone()).collect();
        to.sort();
        assert_eq!(to, vec!["lan:a".to_string(), "lan:b".to_string()]);
        assert!(matches!(
            &h.sent_to(TransportKind::Lan, "lan:a")[0],
            RoomMessage::Emote { d: 1, e, x, y, .. } if e.as_str() == "gg" && *x == 0.1 && *y == 0.2
        ));
        h.cmd(RoomCommand::SetFlags { paused: true, room_muted: false });
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::SendEmote { number: 1, fx: 0.1, fy: 0.2, reference: "gg".to_string() });
        assert!(h.lan.lock().sent.is_empty());
        h.cmd(RoomCommand::SetFlags { paused: false, room_muted: true });
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::SendEmote { number: 1, fx: 0.1, fy: 0.2, reference: "gg".to_string() });
        assert_eq!(h.lan.lock().sent.len(), 2);
    }

    #[test]
    fn sends_a_ping_to_each_member_over_its_path_and_nothing_when_idle_or_paused() {
        let mut h = Harness::new(true);
        h.cmd(RoomCommand::SendPing { number: 1, fx: 0.1, fy: 0.2, id: "omw".to_string() });
        assert!(h.lan.lock().sent.is_empty());
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "A", 1);
        lan_remote("2222222222222222", "lan:b").presence(&mut h, "B", 1);
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::SendPing { number: 1, fx: 0.1, fy: 0.2, id: "omw".to_string() });
        let mut to: Vec<String> = h.lan.lock().sent.iter().map(|(p, _)| p.clone()).collect();
        to.sort();
        assert_eq!(to, vec!["lan:a".to_string(), "lan:b".to_string()]);
        assert!(matches!(
            &h.sent_to(TransportKind::Lan, "lan:a")[0],
            RoomMessage::Ping { d: 1, ping, x, y, .. } if ping.as_str() == "omw" && *x == 0.1 && *y == 0.2
        ));
        h.cmd(RoomCommand::SetFlags { paused: true, room_muted: false });
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::SendPing { number: 1, fx: 0.1, fy: 0.2, id: "omw".to_string() });
        assert!(h.lan.lock().sent.is_empty());
    }

    #[test]
    fn announces_a_status_change_right_away() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::SetFlags { paused: true, room_muted: false });
        let sent = h.all_sent(TransportKind::Lan);
        assert_eq!(sent.len(), 1);
        assert!(matches!(sent[0], RoomMessage::Presence { status: Status::Paused, .. }));
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::SetFlags { paused: false, room_muted: true });
        let sent = h.all_sent(TransportKind::Lan);
        assert_eq!(sent.len(), 1);
        assert!(matches!(sent[0], RoomMessage::Presence { status: Status::Muted, .. }));
    }

    #[test]
    fn also_sends_presence_and_bye_straight_to_known_members() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "A", 1);
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::SetFlags { paused: true, room_muted: false });
        let sent = h.sent_to(TransportKind::Lan, "lan:a");
        assert_eq!(sent.len(), 1);
        assert!(matches!(sent[0], RoomMessage::Presence { status: Status::Paused, .. }));
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::Leave);
        assert!(matches!(h.sent_to(TransportKind::Lan, "lan:a")[0], RoomMessage::Bye { .. }));
    }

    #[test]
    fn announces_on_a_transport_as_soon_as_it_comes_up() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        h.clear(TransportKind::Lan);
        h.set_link(TransportKind::Lan, Link::Connecting);
        h.set_link(TransportKind::Lan, Link::Ok);
        let sent = h.all_sent(TransportKind::Lan);
        assert_eq!(sent.len(), 1);
        assert!(matches!(sent[0], RoomMessage::Presence { .. }));
    }

    #[test]
    fn re_announces_every_2s_and_after_a_profile_change() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        h.clear(TransportKind::Lan);
        h.advance(2000);
        assert!(h.all_sent(TransportKind::Lan).iter().filter(|m| matches!(m, RoomMessage::Presence { .. })).count() >= 1);
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::Profile(Profile { name: "Renamed".to_string(), color: 1, limit: 5 }));
        let sent = h.all_sent(TransportKind::Lan);
        assert_eq!(sent.len(), 1);
        assert!(matches!(&sent[0], RoomMessage::Presence { name, .. } if name == "Renamed"));
    }

    #[test]
    fn leave_says_bye_stops_and_forgets_the_code_while_quit_keeps_it() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        h.clear(TransportKind::Lan);
        h.cmd(RoomCommand::Leave);
        assert!(matches!(h.all_sent(TransportKind::Lan)[0], RoomMessage::Bye { .. }));
        assert!(h.lan.lock().started.is_none());
        assert_eq!(h.saved.last(), Some(&None));
        let state = h.state();
        assert_eq!(state.code, None);
        assert!(state.members.is_empty());

        h.join_active(&code());
        let before = h.saved.len();
        h.cmd(RoomCommand::Quit);
        assert_eq!(h.saved.len(), before);
        assert!(h.state().members.is_empty());
    }

    #[test]
    fn uses_a_new_peer_id_on_every_join() {
        let mut h = Harness::new(true);
        h.join(&code());
        let first = h.all_sent(TransportKind::Lan).last().unwrap().peer().to_string();
        h.cmd(RoomCommand::Leave);
        h.clear(TransportKind::Lan);
        h.join(&code());
        let second = h.all_sent(TransportKind::Lan)[0].clone();
        assert_ne!(second.peer(), first);
        assert_eq!(second.seq(), 1);
    }

    #[test]
    fn a_leave_right_after_a_join_wins() {
        let mut h = Harness::new(true);
        h.cmd(RoomCommand::Join(code()));
        h.cmd(RoomCommand::Leave);
        h.advance(3000);
        assert!(h.state().members.is_empty());
        assert!(h.lan.lock().started.is_none());
    }

    #[test]
    fn uses_lan_while_it_is_alive_falls_back_to_the_internet_and_returns_to_lan() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Alex", 1);
        alex.presence_on(&mut h, TransportKind::Internet, "net:x", "Alex", 1);
        assert_eq!(h.state().members[1].path, Path::Lan);

        h.advance(6000);
        alex.presence_on(&mut h, TransportKind::Internet, "net:x", "Alex", 1);
        assert_eq!(h.state().members[1].path, Path::Internet);
        h.clear(TransportKind::Lan);
        h.clear(TransportKind::Internet);
        h.cmd(RoomCommand::SendPing { number: 1, fx: 0.0, fy: 0.0, id: "omw".to_string() });
        assert!(h.lan.lock().sent.is_empty());
        assert_eq!(h.net.lock().sent.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), vec!["*".to_string()]);

        alex.presence(&mut h, "Alex", 1);
        assert_eq!(h.state().members[1].path, Path::Lan);
    }

    #[test]
    fn path_is_connecting_for_a_lan_member_we_stop_hearing_before_20s() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "Alex", 2);
        h.advance(6000);
        assert_eq!(h.state().members[1].path, Path::Connecting);
        h.advance(9000);
        assert_eq!(h.state().members.len(), 1);
    }

    #[test]
    fn path_is_unreachable_for_an_internet_member_we_stop_hearing_after_20s() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        net_remote("2222222222222222", "net:b").presence(&mut h, "B", 3);
        h.advance(25_000);
        assert_eq!(h.state().members[1].path, Path::Unreachable);
    }

    #[test]
    fn becomes_lonely_20s_after_the_last_member_leaves_and_clears_when_one_joins() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        assert!(!h.state().lonely);
        h.advance(17_000);
        assert!(h.state().lonely);

        lan_remote("1111111111111111", "lan:a").presence(&mut h, "Alex", 2);
        assert!(!h.state().lonely);
        h.advance(15_000);
        assert_eq!(h.state().members.len(), 1);
        assert!(!h.state().lonely);
        h.advance(19_999);
        assert!(!h.state().lonely);
        h.advance(1);
        assert!(h.state().lonely);
        lan_remote("2222222222222222", "lan:b").presence(&mut h, "Bob", 2);
        assert!(!h.state().lonely);
    }

    #[test]
    fn warns_about_a_clock_more_than_10_minutes_off_without_adding_anyone_then_clears() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut late = lan_remote("1111111111111111", "lan:a");
        late.stale_presence(&mut h, "Old");
        late.stale_presence(&mut h, "Old");
        assert!(!h.state().clock_skew);
        late.stale_presence(&mut h, "Old");
        assert!(h.state().clock_skew);
        assert_eq!(h.state().members.len(), 1);
        h.advance(30_000);
        assert!(!h.state().clock_skew);
    }

    #[test]
    fn shows_a_ping_that_arrives_over_both_transports_once() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        let mut alex = lan_remote("1111111111111111", "lan:a");
        alex.presence(&mut h, "Alex", 1);
        let seq = alex.next();
        let msg = RoomMessage::Ping { peer: alex.peer.clone(), seq, ts: h.now() as u64, ping: "push".to_string(), d: 1, x: 0.5, y: 0.5 };
        let bytes = seal(&KEY, &encode_message(&msg));
        h.deliver(TransportKind::Lan, "lan:a", bytes.clone());
        h.deliver(TransportKind::Internet, "net:x", bytes);
        assert_eq!(h.pings.len(), 1);
    }

    #[test]
    fn unicasts_presence_only_on_lan() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "A", 1);
        net_remote("2222222222222222", "net:x").presence(&mut h, "B", 1);
        h.advance(1000);
        h.clear(TransportKind::Lan);
        h.clear(TransportKind::Internet);
        h.cmd(RoomCommand::Profile(Profile { name: "Me".to_string(), color: 1, limit: 5 }));
        let mut to: Vec<String> = h.lan.lock().sent.iter().map(|(p, _)| p.clone()).collect();
        to.sort();
        assert_eq!(to, vec!["*".to_string(), "lan:a".to_string()]);
        assert_eq!(h.net.lock().sent.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), vec!["*".to_string()]);
    }

    #[test]
    fn sends_a_ping_once_over_a_shared_transport_however_many_members_use_it() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        net_remote("1111111111111111", "net:a").presence(&mut h, "A", 3);
        net_remote("2222222222222222", "net:b").presence(&mut h, "B", 3);
        h.clear(TransportKind::Internet);
        h.cmd(RoomCommand::SendPing { number: 1, fx: 0.5, fy: 0.5, id: "omw".to_string() });
        assert_eq!(h.net.lock().sent.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), vec!["*".to_string()]);
        assert!(matches!(h.all_sent(TransportKind::Internet)[0], RoomMessage::Ping { .. }));
    }

    #[test]
    fn caps_pings_over_a_shared_transport_at_5_per_s_with_bursts_of_10_while_lan_members_get_every_ping() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "A", 1);
        net_remote("2222222222222222", "net:b").presence(&mut h, "B", 1);
        h.clear(TransportKind::Lan);
        h.clear(TransportKind::Internet);
        for _ in 0..15 {
            h.cmd(RoomCommand::SendPing { number: 1, fx: 0.5, fy: 0.5, id: "omw".to_string() });
        }
        assert_eq!(h.sent_to(TransportKind::Lan, "lan:a").len(), 15);
        assert_eq!(h.all_sent(TransportKind::Internet).iter().filter(|m| matches!(m, RoomMessage::Ping { .. })).count(), 10);
        h.clear(TransportKind::Internet);
        h.advance(1000);
        for _ in 0..15 {
            h.cmd(RoomCommand::SendPing { number: 1, fx: 0.5, fy: 0.5, id: "omw".to_string() });
        }
        assert_eq!(h.all_sent(TransportKind::Internet).iter().filter(|m| matches!(m, RoomMessage::Ping { .. })).count(), 5);
    }

    #[test]
    fn sends_an_emote_once_over_a_shared_transport_and_pings_and_emotes_share_one_cap() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "A", 1);
        net_remote("2222222222222222", "net:b").presence(&mut h, "B", 1);
        net_remote("3333333333333333", "net:c").presence(&mut h, "C", 1);
        h.clear(TransportKind::Lan);
        h.clear(TransportKind::Internet);
        h.cmd(RoomCommand::SendEmote { number: 1, fx: 0.5, fy: 0.5, reference: "gg".to_string() });
        assert_eq!(h.net.lock().sent.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>(), vec!["*".to_string()]);
        assert!(matches!(h.all_sent(TransportKind::Internet)[0], RoomMessage::Emote { .. }));

        h.clear(TransportKind::Internet);
        h.advance(1000);
        for i in 0..15 {
            if i % 2 == 1 {
                h.cmd(RoomCommand::SendPing { number: 1, fx: 0.5, fy: 0.5, id: "omw".to_string() });
            } else {
                h.cmd(RoomCommand::SendEmote { number: 1, fx: 0.5, fy: 0.5, reference: "gg".to_string() });
            }
        }
        let kinds: Vec<&str> = h
            .all_sent(TransportKind::Internet)
            .iter()
            .filter_map(|m| match m {
                RoomMessage::Ping { .. } => Some("ping"),
                RoomMessage::Emote { .. } => Some("emote"),
                _ => None,
            })
            .collect();
        assert_eq!(kinds, vec!["emote", "ping", "emote", "ping", "emote", "ping", "emote", "ping", "emote", "ping"]);
        let to_lan = h.sent_to(TransportKind::Lan, "lan:a");
        assert_eq!(to_lan.iter().filter(|m| matches!(m, RoomMessage::Ping { .. } | RoomMessage::Emote { .. })).count(), 16);
        h.clear(TransportKind::Internet);
        h.advance(1000);
        for _ in 0..15 {
            h.cmd(RoomCommand::SendEmote { number: 1, fx: 0.5, fy: 0.5, reference: "gg".to_string() });
        }
        assert_eq!(h.all_sent(TransportKind::Internet).iter().filter(|m| matches!(m, RoomMessage::Emote { .. })).count(), 5);
    }

    #[test]
    fn never_caps_presence_or_bye_over_a_shared_transport() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        net_remote("2222222222222222", "net:b").presence(&mut h, "B", 3);
        for _ in 0..20 {
            h.cmd(RoomCommand::SendPing { number: 1, fx: 0.5, fy: 0.5, id: "omw".to_string() });
        }
        h.advance(1000);
        h.clear(TransportKind::Internet);
        h.cmd(RoomCommand::SetFlags { paused: false, room_muted: true });
        h.cmd(RoomCommand::Leave);
        let kinds: Vec<&str> = h
            .all_sent(TransportKind::Internet)
            .iter()
            .map(|m| match m {
                RoomMessage::Presence { .. } => "presence",
                RoomMessage::Bye { .. } => "bye",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["presence", "bye"]);
    }

    #[test]
    fn answers_link_up_and_new_members_at_once_then_at_most_once_a_second_with_the_latest_state() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        h.advance(1500);
        h.clear(TransportKind::Internet);
        h.link_up(TransportKind::Internet);
        assert_eq!(h.all_sent(TransportKind::Internet).len(), 1);
        net_remote("2222222222222222", "net:b").presence(&mut h, "B", 3);
        h.cmd(RoomCommand::Profile(Profile { name: "Later".to_string(), color: 1, limit: 5 }));
        assert_eq!(h.all_sent(TransportKind::Internet).len(), 1);
        h.advance(999);
        assert_eq!(h.all_sent(TransportKind::Internet).len(), 1);
        h.advance(1);
        let names: Vec<String> = h
            .all_sent(TransportKind::Internet)
            .iter()
            .filter_map(|m| match m {
                RoomMessage::Presence { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["Me".to_string(), "Later".to_string()]);
    }

    #[test]
    fn joining_over_relays_only_announces_as_soon_as_the_first_relay_is_live() {
        let mut h = Harness::new(true);
        h.net.lock().start_link = Link::Connecting;
        h.join(&code());
        h.advance(300);
        h.clear(TransportKind::Internet);
        h.link_up(TransportKind::Internet);
        h.advance(700);
        assert!(h.all_sent(TransportKind::Internet).iter().any(|m| matches!(m, RoomMessage::Presence { .. })));
    }

    #[test]
    fn settles_3s_after_the_internet_transport_leaves_starting() {
        let mut h = Harness::new(true);
        h.net.lock().start_link = Link::Connecting;
        h.join(&code());
        h.advance(5000);
        assert_eq!(h.state().code.as_deref(), Some(&format_room_code(&code())[..]));
        assert!(h.state().members.len() == 1);
        assert!(h.saved.is_empty());
        h.net.lock().start_link = Link::Ok;
        h.set_link(TransportKind::Internet, Link::Ok);
        h.advance(2999);
        assert!(h.saved.is_empty());
        h.advance(1);
        assert_eq!(h.saved.len(), 1);
    }

    #[test]
    fn settles_at_the_15s_cap_when_the_internet_never_leaves_starting() {
        let mut h = Harness::new(true);
        h.net.lock().start_link = Link::Connecting;
        h.join(&code());
        h.advance(14_999);
        assert!(h.saved.is_empty());
        h.advance(1);
        assert_eq!(h.saved.len(), 1);
    }

    #[test]
    fn never_settles_later_than_the_cap_when_the_internet_comes_up_late() {
        let mut h = Harness::new(true);
        h.net.lock().start_link = Link::Connecting;
        h.join(&code());
        h.advance(13_000);
        h.set_link(TransportKind::Internet, Link::Ok);
        h.advance(2000);
        assert_eq!(h.saved.len(), 1);
    }

    #[test]
    fn settles_after_3s_when_internet_is_not_allowed() {
        let mut h = Harness::new(true);
        h.cmd(RoomCommand::SetInternet(false));
        h.net.lock().start_link = Link::Connecting;
        h.join(&code());
        assert_eq!(h.net.lock().starts, 0);
        h.advance(3000);
        assert_eq!(h.saved.len(), 1);
        assert_eq!(h.state().internet, Link::Unavailable);
    }

    #[test]
    fn keeps_an_internet_path_alive_for_25s_and_drops_an_internet_member_after_30s() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        net_remote("2222222222222222", "net:b").presence(&mut h, "B", 3);
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "A", 1);
        h.advance(24_000);
        assert_eq!(h.state().members.iter().find(|m| m.peer == "2222222222222222").map(|m| m.path), Some(Path::Internet));
        assert!(h.state().members.iter().all(|m| m.peer != "1111111111111111"));
        h.advance(5000);
        assert_eq!(h.state().members.len(), 2);
        h.advance(1000);
        assert_eq!(h.state().members.len(), 1);
        assert_eq!(h.toasts.iter().filter(|t| matches!(t, Toast::Lost { .. })).count(), 2);
    }

    #[test]
    fn disabling_internet_mid_room_stops_the_relay_transport_without_a_bye_and_forgets_internet_paths() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        net_remote("2222222222222222", "net:b").presence(&mut h, "B", 3);
        h.clear(TransportKind::Internet);
        h.cmd(RoomCommand::SetInternet(false));
        assert!(h.net.lock().sent.is_empty());
        assert!(h.net.lock().started.is_none());
        assert_eq!(h.state().internet, Link::Unavailable);
        assert_ne!(h.state().members[1].path, Path::Internet);
        h.cmd(RoomCommand::SendPing { number: 1, fx: 0.5, fy: 0.5, id: "omw".to_string() });
        assert!(h.net.lock().sent.is_empty());
    }

    #[test]
    fn enabling_internet_mid_room_starts_it_with_the_current_keys_and_announces() {
        let mut h = Harness::new(true);
        h.cmd(RoomCommand::SetInternet(false));
        h.join_active(&code());
        assert_eq!(h.net.lock().starts, 0);
        h.cmd(RoomCommand::SetInternet(true));
        assert_eq!(h.net.lock().started.as_ref().unwrap().msg_key, KEY);
        assert_eq!(h.all_sent(TransportKind::Internet).len(), 1);
        assert!(matches!(h.all_sent(TransportKind::Internet)[0], RoomMessage::Presence { .. }));
        assert_eq!(h.state().internet, Link::Ok);
    }

    #[test]
    fn toggling_internet_off_on_off_leaves_no_live_relay_transport() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        h.cmd(RoomCommand::SetInternet(false));
        h.cmd(RoomCommand::SetInternet(true));
        h.cmd(RoomCommand::SetInternet(false));
        let net = h.net.lock();
        assert_eq!(net.starts - net.stops, 0);
        assert!(net.started.is_none());
        assert_eq!(h.state().internet, Link::Unavailable);
    }

    #[test]
    fn wake_pokes_every_active_transport_and_refreshes_the_grace_period() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        lan_remote("1111111111111111", "lan:a").presence(&mut h, "Alex", 2);
        h.jump(60_000);
        h.cmd(RoomCommand::Wake);
        assert_eq!(h.lan.lock().wakes, 1);
        assert_eq!(h.net.lock().wakes, 1);
        h.advance(1000);
        assert_eq!(h.state().members.len(), 2);
    }

    #[test]
    fn retry_internet_pokes_only_the_internet_transport() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        h.cmd(RoomCommand::RetryInternet);
        assert_eq!(h.net.lock().retries, 1);
        assert_eq!(h.lan.lock().retries, 0);
        h.cmd(RoomCommand::SetInternet(false));
        h.cmd(RoomCommand::RetryInternet);
        assert_eq!(h.net.lock().retries, 1);
    }

    #[test]
    fn quit_ends_both_transport_threads_after_their_drain() {
        let mut h = Harness::new(true);
        h.join_active(&code());
        h.cmd(RoomCommand::Quit);
        assert_eq!(h.lan.lock().shutdowns, 1);
        assert_eq!(h.net.lock().shutdowns, 1);
        assert!(h.lan.lock().started.is_none());
        assert!(h.net.lock().started.is_none());
    }

    #[test]
    fn binds_lan_on_a_room_start_when_the_port_was_taken_at_process_start() {
        use std::net::UdpSocket;
        use std::time::Instant;

        fn wait_until(deadline: Instant, mut done: impl FnMut() -> bool) -> bool {
            while Instant::now() < deadline {
                if done() {
                    return true;
                }
                thread::sleep(Duration::from_millis(10));
            }
            done()
        }

        let held = UdpSocket::bind(("0.0.0.0", crate::room::lan::LAN_PORT)).expect("hold the lan port");
        let (events, _rx) = channel();
        let room = Room::start(
            Profile { name: "Me".to_string(), color: 1, limit: 5 },
            "0.3.0".to_string(),
            false,
            Vec::new(),
            events,
        );
        room.send(RoomCommand::Create);
        assert!(wait_until(Instant::now() + Duration::from_secs(2), || room.state().phase != Phase::Idle));
        assert_eq!(room.state().lan, Link::Unavailable);

        drop(held);
        room.send(RoomCommand::Create);
        assert!(
            wait_until(Instant::now() + Duration::from_secs(5), || room.state().lan == Link::Ok),
            "lan stayed {:?}",
            room.state().lan
        );
        room.shutdown();
    }
}
