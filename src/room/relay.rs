use std::collections::{HashSet, VecDeque};
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{client_tls_with_config, Error as WsError, Message, WebSocket};

use crate::room::crypto::RoomKeys;
use crate::room::nostr::{
    event_message, make_key_pair, parse_relay_message, req_message, verify_event, KeyPair, NostrEvent, RelayMessage,
    ROOM_EVENT_KIND,
};
use crate::room::transport::{Link, TransportCommand, TransportEvent, TransportHandle, TransportKind};

// Every member connects to all six, so two members meet while they share one that works.
pub const RELAYS: &[&str] = &[
    "wss://nos.lol",
    "wss://nostr.mom",
    "wss://relay.snort.social",
    "wss://relay.agentry.com",
    "wss://relay.routstr.com",
    "wss://nostr.rblb.it",
];

const SUB_ID: &str = "lolping";
const SINCE_S: u64 = 600;
const CONNECT_MS: u64 = 10_000;
const HEALTH_MS: u64 = 25_000;
const BACKOFF_MIN_MS: u64 = 2_000;
const BACKOFF_MAX_MS: u64 = 60_000;
const BACKOFF_RESET_MS: u64 = 60_000;
const JITTER: f64 = 0.2;
const COOL_MS: u64 = 30_000;
const MAX_MESSAGE: usize = 64 * 1024;
const MAX_PER_SECOND: u32 = 400;
const RETIRE_PREFIXES: [&str; 5] = ["blocked:", "restricted:", "auth-required:", "pow:", "invalid:"];
const UNAVAILABLE_MS: u64 = 15_000;
const DEDUPE: usize = 512;
const BUDGET_PER_SECOND: u32 = 200;
const MAX_CONTENT: usize = 1600;
const DRAIN_MS: u64 = 500;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const TICK: Duration = Duration::from_millis(20);

pub enum SocketEvent {
    Open,
    Text(String),
    Binary,
    Closed,
}

pub trait Socket {
    fn send(&mut self, text: &str) -> bool;
    fn close(&mut self);
    fn poll(&mut self) -> Option<SocketEvent>;
}

pub type SocketFactory = Rc<dyn Fn(&str) -> Option<Box<dyn Socket>>>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RelayState {
    Connecting,
    Live,
    Cooling,
    Down,
    Retired,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Timer {
    Connect,
    Health,
    Cool,
    Backoff,
}

pub enum RelayEvent {
    Event(NostrEvent),
    Echo,
    Live,
    Ok { id: String, ok: bool, message: String },
    State(RelayState),
}

pub struct RelayConnection {
    url: String,
    topic: String,
    pubkey: String,
    pub state: RelayState,
    sock: Option<Box<dyn Socket>>,
    stopped: bool,
    draining: bool,
    timer: Option<(u64, Timer)>,
    attempts: u32,
    echoed: bool,
    healthy_since: Option<u64>,
    closed_once: bool,
    window_start: u64,
    window_count: u32,
    factory: SocketFactory,
    random: fn() -> f64,
}

impl RelayConnection {
    pub fn new(url: &str, topic: &str, pubkey: &str, factory: SocketFactory, random: fn() -> f64) -> Self {
        Self {
            url: url.to_string(),
            topic: topic.to_string(),
            pubkey: pubkey.to_string(),
            state: RelayState::Down,
            sock: None,
            stopped: true,
            draining: false,
            timer: None,
            attempts: 0,
            echoed: false,
            healthy_since: None,
            closed_once: false,
            window_start: 0,
            window_count: 0,
            factory,
            random,
        }
    }

    pub fn healthy(&self) -> bool {
        matches!(self.state, RelayState::Live | RelayState::Cooling) && self.echoed
    }

    pub fn start(&mut self, now: u64, out: &mut Vec<RelayEvent>) {
        if !self.stopped {
            return;
        }
        self.stopped = false;
        self.draining = false;
        self.connect(now, out);
    }

    pub fn stop(&mut self) {
        self.stopped = true;
        self.timer = None;
        self.close_socket();
        self.state = RelayState::Down;
    }

    pub fn publish(&mut self, json: &str) -> bool {
        if self.state != RelayState::Live {
            return false;
        }
        match self.sock.as_mut() {
            Some(sock) => sock.send(json),
            None => false,
        }
    }

    pub fn reconnect_now(&mut self, revive: bool, now: u64, out: &mut Vec<RelayEvent>) {
        if self.stopped || self.draining {
            return;
        }
        if revive {
            if !matches!(self.state, RelayState::Down | RelayState::Retired) {
                return;
            }
        } else if self.state == RelayState::Retired {
            return;
        }
        self.attempts = 0;
        self.connect(now, out);
    }

    pub fn drain(&mut self) {
        if self.stopped {
            return;
        }
        if self.state != RelayState::Live {
            return self.stop();
        }
        self.draining = true;
    }

    pub fn tick(&mut self, now: u64, out: &mut Vec<RelayEvent>) {
        loop {
            let event = match self.sock.as_mut() {
                Some(sock) => sock.poll(),
                None => None,
            };
            match event {
                Some(event) => self.on_socket_event(event, now, out),
                None => break,
            }
        }
        if let Some((deadline, timer)) = self.timer {
            if now < deadline {
                return;
            }
            self.timer = None;
            match timer {
                Timer::Connect | Timer::Health => self.down(now, out),
                Timer::Cool => {
                    self.set_state(RelayState::Live, out);
                    self.arm_health(now);
                }
                Timer::Backoff => self.connect(now, out),
            }
        }
    }

    fn connect(&mut self, now: u64, out: &mut Vec<RelayEvent>) {
        self.timer = None;
        self.close_socket();
        self.echoed = false;
        self.healthy_since = None;
        self.closed_once = false;
        let Some(sock) = (self.factory)(&self.url) else {
            return self.retire(out); // a malformed URL never gets better
        };
        self.sock = Some(sock);
        self.set_state(RelayState::Connecting, out);
        self.timer = Some((now + CONNECT_MS, Timer::Connect));
    }

    fn subscribe(&mut self, now: u64) {
        let since = (now / 1000).saturating_sub(SINCE_S);
        let request = req_message(SUB_ID, &self.topic, since);
        if let Some(sock) = self.sock.as_mut() {
            sock.send(&request);
        }
    }

    fn on_socket_event(&mut self, event: SocketEvent, now: u64, out: &mut Vec<RelayEvent>) {
        match event {
            SocketEvent::Open => {
                self.subscribe(now);
                self.set_state(RelayState::Live, out);
                self.arm_health(now);
                if !self.stopped {
                    out.push(RelayEvent::Live);
                }
            }
            SocketEvent::Text(text) => self.on_message(&text, now, out),
            SocketEvent::Binary => self.retire(out),
            SocketEvent::Closed => self.down(now, out),
        }
    }

    fn on_message(&mut self, text: &str, now: u64, out: &mut Vec<RelayEvent>) {
        if now.saturating_sub(self.window_start) >= 1000 {
            self.window_start = now;
            self.window_count = 0;
        }
        self.window_count += 1;
        if self.window_count > MAX_PER_SECOND {
            return self.retire(out);
        }
        if text.len() > MAX_MESSAGE {
            return self.retire(out);
        }
        let Some(message) = parse_relay_message(text) else { return };
        match message {
            RelayMessage::Event { sub_id, event } => {
                if sub_id != SUB_ID || event.kind != ROOM_EVENT_KIND {
                    return;
                }
                let tagged = event
                    .tags
                    .iter()
                    .any(|tag| tag.first().map(String::as_str) == Some("x") && tag.get(1).map(String::as_str) == Some(&self.topic));
                if !tagged {
                    return;
                }
                if event.pubkey == self.pubkey {
                    self.on_echo(now, out);
                } else {
                    out.push(RelayEvent::Event(event));
                }
            }
            RelayMessage::Ok { event_id, ok, message } => {
                let cooling = !ok && message.starts_with("rate-limited:");
                let retiring = !ok && !cooling && RETIRE_PREFIXES.iter().any(|prefix| message.starts_with(prefix));
                out.push(RelayEvent::Ok { id: event_id, ok, message });
                if cooling {
                    self.cool(now, out);
                } else if retiring {
                    self.retire(out);
                }
            }
            RelayMessage::Closed { sub_id, .. } if sub_id == SUB_ID => {
                if self.closed_once {
                    self.retire(out);
                } else {
                    self.closed_once = true;
                    self.subscribe(now);
                }
            }
            _ => {}
        }
    }

    fn on_echo(&mut self, now: u64, out: &mut Vec<RelayEvent>) {
        if !matches!(self.state, RelayState::Live | RelayState::Cooling) {
            return;
        }
        if !self.echoed {
            self.echoed = true;
            self.healthy_since = Some(now);
        }
        out.push(RelayEvent::Echo);
        if self.state == RelayState::Live {
            self.arm_health(now);
        }
    }

    fn cool(&mut self, now: u64, out: &mut Vec<RelayEvent>) {
        if self.state != RelayState::Live {
            return;
        }
        if self.draining {
            return self.stop();
        }
        self.timer = None;
        self.set_state(RelayState::Cooling, out);
        self.timer = Some((now + COOL_MS, Timer::Cool));
    }

    fn arm_health(&mut self, now: u64) {
        self.timer = Some((now + HEALTH_MS, Timer::Health));
    }

    fn down(&mut self, now: u64, out: &mut Vec<RelayEvent>) {
        if self.stopped || self.state == RelayState::Retired {
            return;
        }
        if self.draining {
            return self.stop();
        }
        if self.healthy_since.is_some_and(|since| now.saturating_sub(since) >= BACKOFF_RESET_MS) {
            self.attempts = 0;
        }
        self.timer = None;
        self.close_socket();
        self.echoed = false;
        self.healthy_since = None;
        self.set_state(RelayState::Down, out);
        let delay = BACKOFF_MIN_MS.saturating_mul(1u64 << self.attempts.min(20)).min(BACKOFF_MAX_MS);
        self.attempts += 1;
        let jittered = delay as f64 * (1.0 + ((self.random)() * 2.0 - 1.0) * JITTER);
        self.timer = Some((now + jittered as u64, Timer::Backoff));
    }

    fn retire(&mut self, out: &mut Vec<RelayEvent>) {
        self.timer = None;
        self.close_socket();
        self.echoed = false;
        self.set_state(RelayState::Retired, out);
    }

    fn close_socket(&mut self) {
        if let Some(mut sock) = self.sock.take() {
            sock.close();
        }
    }

    fn set_state(&mut self, state: RelayState, out: &mut Vec<RelayEvent>) {
        if state == self.state {
            return;
        }
        self.state = state;
        if !self.stopped {
            out.push(RelayEvent::State(state));
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PoolStatus {
    Off,
    Starting,
    Ok,
    Unavailable,
}

pub enum PoolEvent {
    Event(NostrEvent),
    Status(PoolStatus),
    LinkUp(usize),
    Ok { id: String, ok: bool },
}

pub struct RelayPool {
    pub status: PoolStatus,
    conns: Vec<RelayConnection>,
    ever_live: Vec<bool>,
    failed: Vec<bool>,
    stopped: bool,
    timer: Option<u64>,
    seen: HashSet<String>,
    ring: VecDeque<String>,
    window_start: u64,
    window_count: u32,
}

impl RelayPool {
    pub fn new(urls: &[&str], topic: &str, pubkey: &str, factory: SocketFactory, random: fn() -> f64) -> Self {
        let conns: Vec<RelayConnection> = urls
            .iter()
            .map(|url| RelayConnection::new(url, topic, pubkey, Rc::clone(&factory), random))
            .collect();
        let count = conns.len();
        Self {
            status: PoolStatus::Off,
            conns,
            ever_live: vec![false; count],
            failed: vec![false; count],
            stopped: true,
            timer: None,
            seen: HashSet::new(),
            ring: VecDeque::new(),
            window_start: 0,
            window_count: 0,
        }
    }

    pub fn start(&mut self, now: u64, out: &mut Vec<PoolEvent>) {
        if !self.stopped {
            return;
        }
        self.stopped = false;
        self.set_status(PoolStatus::Starting, out);
        self.timer = Some(now + UNAVAILABLE_MS);
        for index in 0..self.conns.len() {
            let mut events = Vec::new();
            self.conns[index].start(now, &mut events);
            for event in events {
                self.on_conn_event(index, event, now, out);
            }
        }
    }

    pub fn stop(&mut self) {
        self.stopped = true;
        self.timer = None;
        for conn in &mut self.conns {
            conn.stop();
        }
        self.status = PoolStatus::Off;
    }

    pub fn publish(&mut self, event: &NostrEvent) -> usize {
        let json = event_message(event);
        let mut sent = 0;
        for conn in &mut self.conns {
            if conn.publish(&json) {
                sent += 1;
            }
        }
        sent
    }

    pub fn drain(&mut self) -> usize {
        for conn in &mut self.conns {
            conn.drain();
        }
        self.conns.iter().filter(|conn| conn.state == RelayState::Live).count()
    }

    pub fn retry_now(&mut self, now: u64, out: &mut Vec<PoolEvent>) {
        self.reconnect_all(true, now, out);
    }

    pub fn wake(&mut self, now: u64, out: &mut Vec<PoolEvent>) {
        self.reconnect_all(false, now, out);
    }

    pub fn tick(&mut self, now: u64, out: &mut Vec<PoolEvent>) {
        for index in 0..self.conns.len() {
            let mut events = Vec::new();
            self.conns[index].tick(now, &mut events);
            for event in events {
                self.on_conn_event(index, event, now, out);
            }
        }
        if let Some(deadline) = self.timer {
            if now >= deadline {
                self.timer = None;
                if self.status == PoolStatus::Starting {
                    self.set_status(PoolStatus::Unavailable, out);
                }
            }
        }
    }

    fn reconnect_all(&mut self, revive: bool, now: u64, out: &mut Vec<PoolEvent>) {
        for index in 0..self.conns.len() {
            let mut events = Vec::new();
            self.conns[index].reconnect_now(revive, now, &mut events);
            for event in events {
                self.on_conn_event(index, event, now, out);
            }
        }
    }

    fn on_conn_event(&mut self, index: usize, event: RelayEvent, now: u64, out: &mut Vec<PoolEvent>) {
        match event {
            RelayEvent::Event(event) => self.on_event(event, now, out),
            RelayEvent::Echo => self.refresh(out),
            RelayEvent::State(state) => {
                if matches!(state, RelayState::Down | RelayState::Retired) && !self.ever_live[index] {
                    self.failed[index] = true;
                }
                self.refresh(out);
            }
            RelayEvent::Live => {
                self.ever_live[index] = true;
                out.push(PoolEvent::LinkUp(index));
            }
            RelayEvent::Ok { id, ok, .. } => out.push(PoolEvent::Ok { id, ok }),
        }
    }

    fn on_event(&mut self, event: NostrEvent, now: u64, out: &mut Vec<PoolEvent>) {
        if self.stopped || !self.seen.insert(event.id.clone()) {
            return;
        }
        self.ring.push_back(event.id.clone());
        if self.ring.len() > DEDUPE {
            if let Some(oldest) = self.ring.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        if now.saturating_sub(self.window_start) >= 1000 {
            self.window_start = now;
            self.window_count = 0;
        }
        self.window_count += 1;
        if self.window_count > BUDGET_PER_SECOND {
            return;
        }
        out.push(PoolEvent::Event(event));
    }

    fn refresh(&mut self, out: &mut Vec<PoolEvent>) {
        if self.stopped {
            return;
        }
        let lost = self.status == PoolStatus::Ok
            || (self.status == PoolStatus::Starting && self.failed.iter().all(|failed| *failed));
        if self.conns.iter().any(RelayConnection::healthy) {
            self.set_status(PoolStatus::Ok, out);
        } else if lost {
            self.set_status(PoolStatus::Unavailable, out);
        }
    }

    fn set_status(&mut self, status: PoolStatus, out: &mut Vec<PoolEvent>) {
        if status == self.status {
            return;
        }
        self.status = status;
        out.push(PoolEvent::Status(status));
    }
}

pub enum SessionEvent {
    Packet { peer: String, bytes: Vec<u8> },
    Status(Link),
    LinkUp,
}

struct Draining {
    pool: RelayPool,
    id: String,
    deadline: u64,
}

pub struct RelaySession {
    urls: Vec<String>,
    factory: SocketFactory,
    random: fn() -> f64,
    pool: Option<RelayPool>,
    draining: Vec<Draining>,
    pair: Option<KeyPair>,
    topic: String,
    last_sent: Option<String>,
    status: Link,
}

impl RelaySession {
    pub fn new(urls: &[&str], factory: SocketFactory, random: fn() -> f64) -> Self {
        Self {
            urls: urls.iter().map(|url| url.to_string()).collect(),
            factory,
            random,
            pool: None,
            draining: Vec::new(),
            pair: None,
            topic: String::new(),
            last_sent: None,
            status: Link::Unavailable,
        }
    }

    pub fn status(&self) -> Link {
        self.status
    }

    pub fn closing(&self) -> bool {
        self.pool.is_none() && self.draining.is_empty()
    }

    pub fn start(&mut self, keys: &RoomKeys, now: u64, out: &mut Vec<SessionEvent>) {
        if self.pool.is_some() {
            return;
        }
        let pair = make_key_pair();
        let urls: Vec<&str> = self.urls.iter().map(String::as_str).collect();
        let mut pool = RelayPool::new(&urls, &keys.topic, &pair.pubkey, Rc::clone(&self.factory), self.random);
        self.topic = keys.topic.clone();
        self.pair = Some(pair);
        self.last_sent = None;
        let mut events = Vec::new();
        pool.start(now, &mut events);
        for event in events {
            self.on_pool_event(event, out);
        }
        self.pool = Some(pool);
    }

    pub fn stop(&mut self, now: u64, out: &mut Vec<SessionEvent>) {
        let Some(mut pool) = self.pool.take() else { return };
        self.pair = None;
        let pending = self.last_sent.take();
        self.set_status(Link::Unavailable, out);
        match pending {
            Some(id) if pool.drain() > 0 => self.draining.push(Draining { pool, id, deadline: now + DRAIN_MS }),
            _ => pool.stop(),
        }
    }

    pub fn retry_now(&mut self, now: u64, out: &mut Vec<SessionEvent>) {
        if let Some(pool) = self.pool.as_mut() {
            let mut events = Vec::new();
            pool.retry_now(now, &mut events);
            for event in events {
                self.on_pool_event(event, out);
            }
        }
    }

    pub fn wake(&mut self, now: u64, out: &mut Vec<SessionEvent>) {
        if let Some(pool) = self.pool.as_mut() {
            let mut events = Vec::new();
            pool.wake(now, &mut events);
            for event in events {
                self.on_pool_event(event, out);
            }
        }
    }

    pub fn broadcast(&mut self, packet: &[u8], now: u64) {
        let Some(pair) = self.pair.as_ref() else { return };
        let content = STANDARD.encode(packet);
        let event = pair.sign(ROOM_EVENT_KIND, vec![vec!["x".to_string(), self.topic.clone()]], content, now / 1000);
        let sent = self.pool.as_mut().map_or(0, |pool| pool.publish(&event));
        self.last_sent = if sent > 0 { Some(event.id) } else { None };
    }

    pub fn tick(&mut self, now: u64, out: &mut Vec<SessionEvent>) {
        if let Some(mut pool) = self.pool.take() {
            let mut events = Vec::new();
            pool.tick(now, &mut events);
            for event in events {
                self.on_pool_event(event, out);
            }
            self.pool = Some(pool);
        }
        let mut index = 0;
        while index < self.draining.len() {
            let mut events = Vec::new();
            self.draining[index].pool.tick(now, &mut events);
            let mut done = now >= self.draining[index].deadline;
            for event in events {
                if matches!(&event, PoolEvent::Ok { id, .. } if *id == self.draining[index].id) {
                    done = true;
                }
            }
            if done {
                self.draining.remove(index).pool.stop();
            } else {
                index += 1;
            }
        }
    }

    fn on_pool_event(&mut self, event: PoolEvent, out: &mut Vec<SessionEvent>) {
        match event {
            PoolEvent::Event(event) => self.on_event(event, out),
            PoolEvent::Status(status) => self.set_status(link_of(status), out),
            PoolEvent::LinkUp(_) => out.push(SessionEvent::LinkUp),
            PoolEvent::Ok { id, .. } => {
                if self.last_sent.as_deref() == Some(id.as_str()) {
                    self.last_sent = None;
                }
            }
        }
    }

    fn on_event(&mut self, event: NostrEvent, out: &mut Vec<SessionEvent>) {
        if !verify_event(&event) {
            return;
        }
        if event.content.len() > MAX_CONTENT {
            return;
        }
        let Ok(bytes) = STANDARD.decode(&event.content) else { return };
        out.push(SessionEvent::Packet { peer: format!("net:{}", event.pubkey), bytes });
    }

    fn set_status(&mut self, status: Link, out: &mut Vec<SessionEvent>) {
        if status == self.status {
            return;
        }
        self.status = status;
        out.push(SessionEvent::Status(status));
    }
}

fn link_of(status: PoolStatus) -> Link {
    match status {
        PoolStatus::Starting => Link::Connecting,
        PoolStatus::Ok => Link::Ok,
        PoolStatus::Off | PoolStatus::Unavailable => Link::Unavailable,
    }
}

pub fn spawn<S: AsRef<str>>(urls: impl IntoIterator<Item = S>, events: Sender<TransportEvent>) -> TransportHandle {
    let urls: Vec<String> = urls.into_iter().map(|url| url.as_ref().to_string()).collect();
    let (commands, inbox) = mpsc::channel();
    let join = thread::spawn(move || run(urls, inbox, events));
    TransportHandle::new(TransportKind::Internet, true, commands, join)
}

fn run(urls: Vec<String>, commands: Receiver<TransportCommand>, events: Sender<TransportEvent>) {
    let pinned: Vec<&str> = urls.iter().map(String::as_str).collect();
    let mut session = RelaySession::new(&pinned, Rc::new(connect_socket), random);
    let mut out = Vec::new();
    loop {
        match commands.recv_timeout(TICK) {
            Ok(TransportCommand::Start { keys }) => session.start(&keys, now_ms(), &mut out),
            Ok(TransportCommand::SendTo { bytes, .. }) | Ok(TransportCommand::Broadcast { bytes }) => {
                session.broadcast(&bytes, now_ms())
            }
            Ok(TransportCommand::Stop) => {
                session.stop(now_ms(), &mut out);
                if !drain(&mut session, &events, &mut out) {
                    return;
                }
            }
            Ok(TransportCommand::Wake) => session.wake(now_ms(), &mut out),
            Ok(TransportCommand::Retry) => session.retry_now(now_ms(), &mut out),
            Ok(TransportCommand::Shutdown) => {
                session.stop(now_ms(), &mut out);
                drain(&mut session, &events, &mut out);
                return;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                session.stop(now_ms(), &mut out);
                drain(&mut session, &events, &mut out);
                return;
            }
        }
        session.tick(now_ms(), &mut out);
        if !flush(&events, &mut out) {
            return;
        }
    }
}

// Lets a bye already sent reach the relays; false once nobody is listening any more.
fn drain(session: &mut RelaySession, events: &Sender<TransportEvent>, out: &mut Vec<SessionEvent>) -> bool {
    while !session.closing() {
        thread::sleep(Duration::from_millis(5));
        session.tick(now_ms(), out);
        if !flush(events, out) {
            return false;
        }
    }
    true
}

fn flush(events: &Sender<TransportEvent>, out: &mut Vec<SessionEvent>) -> bool {
    for event in out.drain(..) {
        let converted = match event {
            SessionEvent::Packet { peer, bytes } => TransportEvent::Packet { peer, bytes },
            SessionEvent::Status(link) => TransportEvent::Status(link),
            SessionEvent::LinkUp => TransportEvent::LinkUp,
        };
        if events.send(converted).is_err() {
            return false;
        }
    }
    true
}

struct WsSocket {
    events: Receiver<SocketEvent>,
    commands: Sender<SocketCommand>,
}

enum SocketCommand {
    Send(String),
    Close,
}

impl Socket for WsSocket {
    fn send(&mut self, text: &str) -> bool {
        self.commands.send(SocketCommand::Send(text.to_string())).is_ok()
    }

    fn close(&mut self) {
        let _ = self.commands.send(SocketCommand::Close);
    }

    fn poll(&mut self) -> Option<SocketEvent> {
        match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Disconnected) => Some(SocketEvent::Closed),
            Err(TryRecvError::Empty) => None,
        }
    }
}

fn connect_socket(url: &str) -> Option<Box<dyn Socket>> {
    let (host, port) = split_url(url)?;
    let url = url.to_string();
    let (tx, rx) = mpsc::channel();
    let (commands, inbox) = mpsc::channel();
    thread::spawn(move || reader(url, host, port, tx, inbox));
    Some(Box::new(WsSocket { events: rx, commands }))
}

fn reader(url: String, host: String, port: u16, events: Sender<SocketEvent>, commands: Receiver<SocketCommand>) {
    let Some(mut socket) = dial(&url, &host, port) else {
        let _ = events.send(SocketEvent::Closed);
        return;
    };
    if events.send(SocketEvent::Open).is_err() {
        return;
    }
    loop {
        match commands.try_recv() {
            Ok(SocketCommand::Send(text)) => {
                if socket.send(Message::text(text)).is_err() {
                    break;
                }
            }
            Ok(SocketCommand::Close) => {
                let _ = socket.close(None);
                let _ = socket.flush();
                break;
            }
            Err(TryRecvError::Disconnected) => return,
            Err(TryRecvError::Empty) => {}
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                if events.send(SocketEvent::Text(text.to_string())).is_err() {
                    return;
                }
            }
            Ok(Message::Binary(_)) => {
                let _ = events.send(SocketEvent::Binary);
                return;
            }
            Ok(Message::Close(_)) => {
                let _ = events.send(SocketEvent::Closed);
                return;
            }
            Ok(_) => {
                let _ = socket.flush(); // ping: tungstenite queued the pong
            }
            Err(WsError::Io(error)) if error.kind() == ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(5)),
            Err(_) => {
                let _ = events.send(SocketEvent::Closed);
                return;
            }
        }
    }
    let _ = events.send(SocketEvent::Closed);
}

fn dial(url: &str, host: &str, port: u16) -> Option<WebSocket<MaybeTlsStream<TcpStream>>> {
    let address = (host, port).to_socket_addrs().ok()?.next()?;
    let stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT)).ok()?;
    let (mut socket, _) = client_tls_with_config(url, stream, None, None).ok()?;
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => {
            let _ = stream.set_read_timeout(None);
            let _ = stream.set_write_timeout(None);
            let _ = stream.set_nonblocking(true);
        }
        MaybeTlsStream::Rustls(stream) => {
            let _ = stream.sock.set_read_timeout(None);
            let _ = stream.sock.set_write_timeout(None);
            let _ = stream.sock.set_nonblocking(true);
        }
        _ => {}
    }
    Some(socket)
}

fn split_url(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let default_port = match scheme {
        "wss" => 443,
        "ws" => 80,
        _ => return None,
    };
    let authority = rest.split(['/', '?', '#']).next()?;
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse().ok()?),
        None => (authority, default_port),
    };
    if host.is_empty() {
        return None;
    }
    Some((host.to_string(), port))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn random() -> f64 {
    getrandom::u64().map_or(0.5, |value| (value >> 11) as f64 / (1u64 << 53) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const NOW: u64 = 1_780_000_000_000;
    const URLS: [&str; 3] = ["wss://a.test", "wss://b.test", "wss://c.test"];

    fn topic() -> String {
        "ab".repeat(32)
    }

    fn keys() -> RoomKeys {
        RoomKeys { msg_key: [1u8; 32], topic: "cd".repeat(32) }
    }

    fn relay_event(sub_id: &str, event: &NostrEvent) -> String {
        serde_json::to_string(&("EVENT", sub_id, event)).unwrap()
    }

    fn ok_reply(id: &str, ok: bool, message: &str) -> String {
        serde_json::to_string(&("OK", id, ok, message)).unwrap()
    }

    fn member_event(keys: &KeyPair, content: &str, created_at: u64) -> NostrEvent {
        keys.sign(ROOM_EVENT_KIND, vec![vec!["x".to_string(), topic()]], content.to_string(), created_at)
    }

    fn tagged_event(keys: &KeyPair, topic: &str, content: &str) -> NostrEvent {
        keys.sign(ROOM_EVENT_KIND, vec![vec!["x".to_string(), topic.to_string()]], content.to_string(), 1_700_000_000)
    }

    struct FakeInner {
        url: String,
        sent: Vec<String>,
        queue: VecDeque<SocketEvent>,
        closed: bool,
    }

    #[derive(Clone)]
    struct FakeSocket(Rc<RefCell<FakeInner>>);

    impl FakeSocket {
        fn new(url: &str) -> Self {
            Self(Rc::new(RefCell::new(FakeInner { url: url.to_string(), sent: Vec::new(), queue: VecDeque::new(), closed: false })))
        }

        fn open(&self) {
            self.0.borrow_mut().queue.push_back(SocketEvent::Open);
        }

        fn receive(&self, text: &str) {
            self.0.borrow_mut().queue.push_back(SocketEvent::Text(text.to_string()));
        }

        fn receive_binary(&self) {
            self.0.borrow_mut().queue.push_back(SocketEvent::Binary);
        }

        fn drop_connection(&self) {
            self.0.borrow_mut().queue.push_back(SocketEvent::Closed);
        }

        fn messages(&self) -> Vec<String> {
            self.0.borrow().sent.clone()
        }

        fn closed(&self) -> bool {
            self.0.borrow().closed
        }
    }

    impl Socket for FakeSocket {
        fn send(&mut self, text: &str) -> bool {
            let mut inner = self.0.borrow_mut();
            if inner.closed {
                return false;
            }
            inner.sent.push(text.to_string());
            true
        }

        fn close(&mut self) {
            self.0.borrow_mut().closed = true;
        }

        fn poll(&mut self) -> Option<SocketEvent> {
            self.0.borrow_mut().queue.pop_front()
        }
    }

    fn fake_factory() -> (SocketFactory, Rc<RefCell<Vec<FakeSocket>>>) {
        let sockets = Rc::new(RefCell::new(Vec::new()));
        let recorder = Rc::clone(&sockets);
        let factory: SocketFactory = Rc::new(move |url: &str| {
            let socket = FakeSocket::new(url);
            recorder.borrow_mut().push(socket.clone());
            Some(Box::new(socket) as Box<dyn Socket>)
        });
        (factory, sockets)
    }

    fn published(socket: &FakeSocket) -> Vec<NostrEvent> {
        socket
            .messages()
            .iter()
            .filter_map(|text| {
                let value: serde_json::Value = serde_json::from_str(text).ok()?;
                if value.get(0)?.as_str()? == "EVENT" {
                    serde_json::from_value(value.get(1)?.clone()).ok()
                } else {
                    None
                }
            })
            .collect()
    }

    fn sent_kinds(socket: &FakeSocket) -> Vec<String> {
        socket
            .messages()
            .iter()
            .filter_map(|text| serde_json::from_str::<serde_json::Value>(text).ok()?.get(0)?.as_str().map(str::to_string))
            .collect()
    }

    struct Conn {
        conn: RelayConnection,
        sockets: Rc<RefCell<Vec<FakeSocket>>>,
        now: u64,
        me: KeyPair,
        events: Vec<NostrEvent>,
        echoes: usize,
        states: Vec<RelayState>,
        oks: Vec<(String, bool, String)>,
        lives: usize,
    }

    impl Conn {
        fn new() -> Self {
            let (factory, sockets) = fake_factory();
            let me = make_key_pair();
            let mut conn = RelayConnection::new("wss://r.test", &topic(), &me.pubkey, factory, || 0.5);
            let mut out = Vec::new();
            conn.start(NOW, &mut out);
            let mut harness = Self { conn, sockets, now: NOW, me, events: Vec::new(), echoes: 0, states: Vec::new(), oks: Vec::new(), lives: 0 };
            harness.collect(out);
            harness
        }

        fn collect(&mut self, out: Vec<RelayEvent>) {
            for event in out {
                match event {
                    RelayEvent::Event(event) => self.events.push(event),
                    RelayEvent::Echo => self.echoes += 1,
                    RelayEvent::State(state) => self.states.push(state),
                    RelayEvent::Ok { id, ok, message } => self.oks.push((id, ok, message)),
                    RelayEvent::Live => self.lives += 1,
                }
            }
        }

        fn pump(&mut self) {
            let mut out = Vec::new();
            self.conn.tick(self.now, &mut out);
            self.collect(out);
        }

        fn advance(&mut self, ms: u64) {
            self.now += ms;
            self.pump();
        }

        fn last(&self) -> FakeSocket {
            self.sockets.borrow().last().unwrap().clone()
        }

        fn socket_count(&self) -> usize {
            self.sockets.borrow().len()
        }

        fn open_last(&mut self) {
            self.last().open();
            self.pump();
        }

        fn drop_last(&mut self) {
            self.last().drop_connection();
            self.pump();
        }

        fn receive(&mut self, text: &str) {
            self.last().receive(text);
            self.pump();
        }

        fn echo(&mut self) {
            let event = member_event(&self.me, "AAAA", self.now / 1000);
            self.receive(&relay_event(SUB_ID, &event));
        }

        fn reconnect(&mut self, revive: bool) {
            let mut out = Vec::new();
            self.conn.reconnect_now(revive, self.now, &mut out);
            self.collect(out);
        }
    }

    #[test]
    fn opens_subscribes_and_goes_live() {
        let mut harness = Conn::new();
        assert_eq!(harness.conn.state, RelayState::Connecting);
        let socket = harness.last();
        harness.open_last();
        let request: serde_json::Value = serde_json::from_str(&socket.messages()[0]).unwrap();
        assert_eq!(
            request,
            serde_json::json!(["REQ", "lolping", {"kinds": [24747], "#x": [topic()], "since": NOW / 1000 - 600, "limit": 0}])
        );
        assert_eq!(harness.conn.state, RelayState::Live);
        assert_eq!(harness.lives, 1);
    }

    #[test]
    fn publishes_only_while_live() {
        let mut harness = Conn::new();
        assert!(!harness.conn.publish("[\"EVENT\",1]"));
        harness.open_last();
        assert!(harness.conn.publish("[\"EVENT\",2]"));
        assert_eq!(&harness.last().messages()[1..], ["[\"EVENT\",2]".to_string()]);
    }

    #[test]
    fn separates_echoes_from_other_members_and_ignores_other_subscriptions_kinds_and_topics() {
        let other = make_key_pair();
        let mut harness = Conn::new();
        harness.open_last();
        let theirs = member_event(&other, "AAAA", 1_700_000_000);
        harness.receive(&relay_event(SUB_ID, &theirs));
        harness.echo();
        harness.receive(&relay_event("other-sub", &member_event(&other, "AAAA", 1_700_000_000)));
        harness.receive(&relay_event(SUB_ID, &other.sign(1, Vec::new(), "AAAA".into(), 1_700_000_000)));
        harness.receive(&relay_event(SUB_ID, &tagged_event(&other, &"cd".repeat(32), "AAAA")));
        harness.receive("[\"NOTICE\",\"hi\"]");
        harness.receive("garbage");
        assert_eq!(harness.events, [theirs]);
        assert_eq!(harness.echoes, 1);
    }

    #[test]
    fn is_healthy_after_the_first_echo_and_down_after_25_s_without_one() {
        let mut harness = Conn::new();
        harness.open_last();
        assert!(!harness.conn.healthy());
        harness.echo();
        assert!(harness.conn.healthy());
        harness.advance(24_999);
        assert_eq!(harness.conn.state, RelayState::Live);
        harness.advance(1);
        assert_eq!(harness.conn.state, RelayState::Down);
        assert!(!harness.conn.healthy());
        harness.advance(1_999);
        assert_eq!(harness.socket_count(), 1);
        harness.advance(1);
        assert_eq!(harness.socket_count(), 2);
    }

    #[test]
    fn a_silent_socket_is_closed_and_reconnected_after_25_s_without_an_echo() {
        let mut harness = Conn::new();
        let socket = harness.last();
        harness.open_last();
        harness.advance(25_000);
        assert!(socket.closed());
        harness.advance(2_000);
        assert_eq!(harness.socket_count(), 2);
        harness.open_last();
        assert_eq!(harness.conn.state, RelayState::Live);
    }

    #[test]
    fn backs_off_2_4_8_16_32_60_and_resets_after_60_s_healthy() {
        let mut harness = Conn::new();
        let mut gaps = Vec::new();
        for _ in 0..7 {
            harness.drop_last();
            let before = harness.socket_count();
            let mut waited = 0;
            while harness.socket_count() == before {
                harness.advance(1_000);
                waited += 1_000;
            }
            gaps.push(waited);
        }
        assert_eq!(gaps, [2_000, 4_000, 8_000, 16_000, 32_000, 60_000, 60_000]);
        harness.open_last();
        harness.echo();
        for _ in 0..6 {
            harness.advance(10_000);
            harness.echo();
        }
        harness.drop_last();
        harness.advance(2_000);
        assert_eq!(harness.socket_count(), 9);
    }

    #[test]
    fn counts_a_socket_that_has_not_opened_within_10_s_as_down() {
        let mut harness = Conn::new();
        harness.advance(9_999);
        assert_eq!(harness.conn.state, RelayState::Connecting);
        harness.advance(1);
        assert_eq!(harness.conn.state, RelayState::Down);
        assert!(harness.sockets.borrow()[0].closed());
    }

    #[test]
    fn rate_limited_cools_for_30_s_publishes_nothing_and_is_not_down_for_missing_echoes() {
        let mut harness = Conn::new();
        let socket = harness.last();
        harness.open_last();
        harness.echo();
        harness.receive(&ok_reply(&"e".repeat(64), false, "rate-limited: slow down"));
        assert_eq!(harness.conn.state, RelayState::Cooling);
        assert!(!harness.conn.publish("[\"EVENT\",3]"));
        assert!(published(&socket).is_empty());
        harness.advance(29_999);
        assert_eq!(harness.conn.state, RelayState::Cooling);
        harness.advance(1);
        assert_eq!(harness.conn.state, RelayState::Live);
        assert_eq!(harness.socket_count(), 1);
        harness.advance(25_000);
        assert_eq!(harness.conn.state, RelayState::Down);
    }

    #[test]
    fn retires_on_every_retire_prefix() {
        for prefix in RETIRE_PREFIXES {
            let mut harness = Conn::new();
            let socket = harness.last();
            harness.open_last();
            harness.receive(&ok_reply(&"e".repeat(64), false, &format!("{prefix} no")));
            assert_eq!(harness.conn.state, RelayState::Retired, "{prefix}");
            assert!(socket.closed());
            harness.advance(600_000);
            assert_eq!(harness.socket_count(), 1);
            harness.reconnect(true);
            assert_eq!(harness.socket_count(), 2);
        }
    }

    #[test]
    fn emits_ok_results() {
        let mut harness = Conn::new();
        harness.open_last();
        harness.receive(&ok_reply(&"e".repeat(64), true, ""));
        assert_eq!(harness.oks, [("e".repeat(64), true, String::new())]);
    }

    #[test]
    fn resubscribes_after_one_closed_and_retires_after_a_second() {
        let mut harness = Conn::new();
        let socket = harness.last();
        harness.open_last();
        harness.receive(&serde_json::to_string(&("CLOSED", "lolping", "error: restarting")).unwrap());
        assert_eq!(sent_kinds(&socket).iter().filter(|kind| *kind == "REQ").count(), 2);
        assert_eq!(harness.conn.state, RelayState::Live);
        harness.receive(&serde_json::to_string(&("CLOSED", "lolping", "error: restarting")).unwrap());
        assert_eq!(harness.conn.state, RelayState::Retired);
    }

    #[test]
    fn retires_after_a_message_over_64_kb() {
        let mut harness = Conn::new();
        harness.open_last();
        harness.receive(&"x".repeat(64 * 1024 + 1));
        assert_eq!(harness.conn.state, RelayState::Retired);
    }

    #[test]
    fn retires_after_more_than_400_messages_in_one_second() {
        let mut harness = Conn::new();
        harness.open_last();
        for _ in 0..400 {
            harness.receive("[\"NOTICE\",\"x\"]");
        }
        assert_eq!(harness.conn.state, RelayState::Live);
        harness.advance(1_000);
        for _ in 0..400 {
            harness.receive("[\"NOTICE\",\"x\"]");
        }
        assert_eq!(harness.conn.state, RelayState::Live);
        harness.receive("[\"NOTICE\",\"x\"]");
        assert_eq!(harness.conn.state, RelayState::Retired);
    }

    #[test]
    fn retires_after_a_binary_frame() {
        let mut harness = Conn::new();
        let socket = harness.last();
        harness.open_last();
        socket.receive_binary();
        harness.pump();
        assert_eq!(harness.conn.state, RelayState::Retired);
    }

    #[test]
    fn reconnect_now_true_leaves_connecting_and_live_relays_alone() {
        let mut harness = Conn::new();
        harness.reconnect(true);
        assert_eq!(harness.socket_count(), 1);
        harness.open_last();
        harness.reconnect(true);
        assert_eq!(harness.socket_count(), 1);
        harness.drop_last();
        harness.reconnect(true);
        assert_eq!(harness.socket_count(), 2);
    }

    #[test]
    fn reconnect_now_false_skips_the_backoff_but_leaves_a_retired_relay_alone() {
        let mut harness = Conn::new();
        harness.drop_last();
        assert_eq!(harness.socket_count(), 1);
        harness.reconnect(false);
        assert_eq!(harness.socket_count(), 2);
        harness.open_last();
        harness.receive(&ok_reply(&"e".repeat(64), false, "blocked: go away"));
        harness.reconnect(false);
        assert_eq!(harness.socket_count(), 2);
    }

    #[test]
    fn stop_closes_the_socket_and_emits_nothing_afterwards() {
        let mut harness = Conn::new();
        let socket = harness.last();
        harness.open_last();
        let states = harness.states.len();
        harness.conn.stop();
        assert!(socket.closed());
        harness.receive(&relay_event(SUB_ID, &member_event(&harness.me, "AAAA", 1_700_000_000)));
        harness.advance(600_000);
        assert!(harness.events.is_empty());
        assert_eq!(harness.states.len(), states);
        assert_eq!(harness.socket_count(), 1);
    }

    struct Pool {
        pool: RelayPool,
        sockets: Rc<RefCell<Vec<FakeSocket>>>,
        now: u64,
        me: KeyPair,
        other: KeyPair,
        events: Vec<NostrEvent>,
        statuses: Vec<PoolStatus>,
        link_ups: Vec<String>,
        oks: Vec<(String, bool)>,
    }

    impl Pool {
        fn new() -> Self {
            let (factory, sockets) = fake_factory();
            let me = make_key_pair();
            let pool = RelayPool::new(&URLS, &topic(), &me.pubkey, factory, || 0.5);
            Self {
                pool,
                sockets,
                now: NOW,
                me,
                other: make_key_pair(),
                events: Vec::new(),
                statuses: Vec::new(),
                link_ups: Vec::new(),
                oks: Vec::new(),
            }
        }

        fn collect(&mut self, out: Vec<PoolEvent>) {
            for event in out {
                match event {
                    PoolEvent::Event(event) => self.events.push(event),
                    PoolEvent::Status(status) => self.statuses.push(status),
                    PoolEvent::LinkUp(index) => self.link_ups.push(URLS[index].to_string()),
                    PoolEvent::Ok { id, ok } => self.oks.push((id, ok)),
                }
            }
        }

        fn start(&mut self) {
            let mut out = Vec::new();
            self.pool.start(self.now, &mut out);
            self.collect(out);
        }

        fn pump(&mut self) {
            let mut out = Vec::new();
            self.pool.tick(self.now, &mut out);
            self.collect(out);
        }

        fn advance(&mut self, ms: u64) {
            self.now += ms;
            self.pump();
        }

        fn sock(&self, url: &str) -> FakeSocket {
            self.sockets.borrow().iter().rfind(|socket| socket.0.borrow().url == url).unwrap().clone()
        }

        fn open(&mut self, url: &str) {
            self.sock(url).open();
            self.pump();
        }

        fn open_all(&mut self) {
            for url in URLS {
                self.sock(url).open();
            }
            self.pump();
        }

        fn echo_from(&mut self, url: &str) {
            let event = member_event(&self.me, "AAAA", self.now / 1000);
            self.sock(url).receive(&relay_event(SUB_ID, &event));
            self.pump();
        }

        fn from(&mut self, url: &str, event: &NostrEvent) {
            self.sock(url).receive(&relay_event(SUB_ID, event));
            self.pump();
        }

        fn theirs(&self, content: &str) -> NostrEvent {
            member_event(&self.other, content, self.now / 1000)
        }
    }

    #[test]
    fn pool_is_off_until_started_then_starting() {
        let mut harness = Pool::new();
        assert_eq!(harness.pool.status, PoolStatus::Off);
        harness.start();
        assert_eq!(harness.pool.status, PoolStatus::Starting);
        assert_eq!(harness.sockets.borrow().iter().map(|s| s.0.borrow().url.clone()).collect::<Vec<_>>(), URLS);
    }

    #[test]
    fn pool_publishes_to_live_relays_only_including_a_fresh_one_before_its_first_echo() {
        let mut harness = Pool::new();
        harness.start();
        harness.open(URLS[0]);
        harness.open(URLS[1]);
        harness.sock(URLS[1]).receive(&ok_reply(&"e".repeat(64), false, "rate-limited: easy"));
        harness.pump();
        let event = member_event(&harness.me, "AAAA", harness.now / 1000);
        assert_eq!(harness.pool.publish(&event), 1);
        assert_eq!(published(&harness.sock(URLS[0])), [event]);
        assert!(published(&harness.sock(URLS[1])).is_empty());
        assert!(harness.sock(URLS[2]).messages().is_empty());
    }

    #[test]
    fn pool_goes_starting_ok_unavailable_and_ok_again() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        harness.echo_from(URLS[0]);
        assert_eq!(harness.pool.status, PoolStatus::Ok);
        harness.sock(URLS[0]).drop_connection();
        harness.pump();
        assert_eq!(harness.pool.status, PoolStatus::Unavailable);
        harness.advance(2_000);
        harness.open(URLS[0]);
        harness.echo_from(URLS[0]);
        assert_eq!(harness.pool.status, PoolStatus::Ok);
        assert_eq!(harness.statuses, [PoolStatus::Starting, PoolStatus::Ok, PoolStatus::Unavailable, PoolStatus::Ok]);
    }

    #[test]
    fn pool_stays_ok_while_any_relay_is_healthy() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        harness.echo_from(URLS[0]);
        harness.echo_from(URLS[1]);
        harness.sock(URLS[0]).drop_connection();
        harness.pump();
        assert_eq!(harness.pool.status, PoolStatus::Ok);
    }

    #[test]
    fn pool_is_unavailable_after_15_s_without_any_echo() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        harness.advance(14_999);
        assert_eq!(harness.pool.status, PoolStatus::Starting);
        harness.advance(1);
        assert_eq!(harness.pool.status, PoolStatus::Unavailable);
    }

    #[test]
    fn pool_is_unavailable_as_soon_as_every_relay_has_failed_to_connect() {
        let mut harness = Pool::new();
        harness.start();
        harness.sock(URLS[0]).drop_connection();
        harness.sock(URLS[1]).drop_connection();
        harness.pump();
        assert_eq!(harness.pool.status, PoolStatus::Starting);
        harness.sock(URLS[2]).drop_connection();
        harness.pump();
        assert_eq!(harness.pool.status, PoolStatus::Unavailable);
        harness.advance(2_000);
        harness.open(URLS[0]);
        harness.echo_from(URLS[0]);
        assert_eq!(harness.pool.status, PoolStatus::Ok);
    }

    #[test]
    fn pool_passes_an_event_that_arrives_from_several_relays_once() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        let event = harness.theirs("AAAA");
        for url in URLS {
            harness.from(url, &event);
        }
        assert_eq!(harness.events, [event]);
    }

    #[test]
    fn pool_forgets_ids_after_512_newer_ones() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        let first = harness.theirs("first");
        harness.from(URLS[0], &first);
        for index in 0..512 {
            if index % 100 == 0 {
                harness.advance(1_000);
            }
            let event = harness.theirs(&format!("n{index}"));
            harness.from(URLS[0], &event);
        }
        harness.advance(1_000);
        harness.from(URLS[1], &first);
        assert_eq!(harness.events.iter().filter(|event| event.id == first.id).count(), 2);
    }

    #[test]
    fn pool_caps_incoming_events_at_200_per_second_across_relays() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        for index in 0..150 {
            let event = harness.theirs(&format!("a{index}"));
            harness.from(URLS[0], &event);
        }
        for index in 0..100 {
            let event = harness.theirs(&format!("b{index}"));
            harness.from(URLS[1], &event);
        }
        assert_eq!(harness.events.len(), 200);
        harness.advance(1_000);
        let event = harness.theirs("later");
        harness.from(URLS[2], &event);
        assert_eq!(harness.events.len(), 201);
    }

    #[test]
    fn pool_does_not_count_duplicates_against_the_budget() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        let event = harness.theirs("AAAA");
        for index in 0..300 {
            harness.from(URLS[index % 3], &event);
        }
        let fresh = harness.theirs("new");
        harness.from(URLS[0], &fresh);
        assert_eq!(harness.events.len(), 2);
    }

    #[test]
    fn pool_emits_link_up_each_time_a_relay_goes_live() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        harness.sock(URLS[1]).drop_connection();
        harness.pump();
        harness.advance(2_000);
        harness.open(URLS[1]);
        assert_eq!(harness.link_ups, [URLS[0], URLS[1], URLS[2], URLS[1]]);
    }

    #[test]
    fn pool_forwards_ok_results() {
        let mut harness = Pool::new();
        harness.start();
        harness.open(URLS[0]);
        harness.sock(URLS[0]).receive(&ok_reply(&"e".repeat(64), true, ""));
        harness.pump();
        assert_eq!(harness.oks, [("e".repeat(64), true)]);
    }

    #[test]
    fn pool_retry_now_during_backoff_keeps_one_socket_per_relay() {
        let mut harness = Pool::new();
        harness.start();
        for socket in harness.sockets.borrow().iter() {
            socket.drop_connection();
        }
        harness.pump();
        for _ in 0..3 {
            let mut out = Vec::new();
            harness.pool.retry_now(harness.now, &mut out);
            harness.collect(out);
        }
        for _ in 0..120 {
            harness.advance(1_000);
        }
        for url in URLS {
            let open = harness.sockets.borrow().iter().filter(|socket| socket.0.borrow().url == url && !socket.closed()).count();
            assert_eq!(open, 1, "{url}");
        }
    }

    #[test]
    fn pool_retry_now_revives_retired_relays_and_wake_does_not() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        harness.sock(URLS[0]).receive(&ok_reply(&"e".repeat(64), false, "blocked: no"));
        harness.pump();
        let count = |harness: &Pool| harness.sockets.borrow().iter().filter(|socket| socket.0.borrow().url == URLS[0]).count();
        let mut out = Vec::new();
        harness.pool.wake(harness.now, &mut out);
        harness.collect(out);
        assert_eq!(count(&harness), 1);
        let mut out = Vec::new();
        harness.pool.retry_now(harness.now, &mut out);
        harness.collect(out);
        assert_eq!(count(&harness), 2);
    }

    #[test]
    fn pool_wake_reconnects_every_relay_that_is_not_retired() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        let mut out = Vec::new();
        harness.pool.wake(harness.now, &mut out);
        harness.collect(out);
        assert_eq!(harness.sockets.borrow().len(), 6);
        assert!(harness.sockets.borrow()[..3].iter().all(|socket| socket.closed()));
    }

    #[test]
    fn pool_stop_closes_every_socket_and_emits_nothing_afterwards() {
        let mut harness = Pool::new();
        harness.start();
        harness.open_all();
        let emitted = harness.statuses.len() + harness.events.len() + harness.link_ups.len();
        harness.pool.stop();
        assert!(harness.sockets.borrow().iter().all(|socket| socket.closed()));
        let event = harness.theirs("AAAA");
        harness.from(URLS[0], &event);
        harness.advance(600_000);
        assert_eq!(harness.statuses.len() + harness.events.len() + harness.link_ups.len(), emitted);
        assert_eq!(harness.pool.status, PoolStatus::Off);
    }

    struct Net {
        session: RelaySession,
        sockets: Rc<RefCell<Vec<FakeSocket>>>,
        now: u64,
        keys: RoomKeys,
        packets: Vec<(String, Vec<u8>)>,
        statuses: Vec<Link>,
        link_ups: usize,
        stopping: bool,
        closed_at: Option<u64>,
    }

    impl Net {
        fn new(urls: &[&str]) -> Self {
            let (factory, sockets) = fake_factory();
            Self {
                session: RelaySession::new(urls, factory, || 0.5),
                sockets,
                now: NOW,
                keys: keys(),
                packets: Vec::new(),
                statuses: Vec::new(),
                link_ups: 0,
                stopping: false,
                closed_at: None,
            }
        }

        fn collect(&mut self, out: Vec<SessionEvent>) {
            for event in out {
                match event {
                    SessionEvent::Packet { peer, bytes } => self.packets.push((peer, bytes)),
                    SessionEvent::Status(link) => self.statuses.push(link),
                    SessionEvent::LinkUp => self.link_ups += 1,
                }
            }
        }

        fn pump(&mut self) {
            let mut out = Vec::new();
            self.session.tick(self.now, &mut out);
            self.collect(out);
            if self.stopping && self.closed_at.is_none() && self.session.closing() {
                self.closed_at = Some(self.now);
            }
        }

        fn advance(&mut self, ms: u64) {
            self.now += ms;
            self.pump();
        }

        fn last(&self) -> FakeSocket {
            self.sockets.borrow().last().unwrap().clone()
        }

        fn start(&mut self) {
            let mut out = Vec::new();
            self.session.start(&self.keys, self.now, &mut out);
            self.collect(out);
        }

        fn stop(&mut self) {
            let mut out = Vec::new();
            self.session.stop(self.now, &mut out);
            self.collect(out);
            self.stopping = true;
            if self.session.closing() {
                self.closed_at = Some(self.now);
            }
        }

        fn open_last(&mut self) {
            self.last().open();
            self.pump();
        }

        fn drop_last(&mut self) {
            self.last().drop_connection();
            self.pump();
        }

        fn receive(&mut self, text: &str) {
            self.last().receive(text);
            self.pump();
        }

        fn broadcast(&mut self, packet: &[u8]) {
            self.session.broadcast(packet, self.now);
        }

        fn retry_now(&mut self) {
            let mut out = Vec::new();
            self.session.retry_now(self.now, &mut out);
            self.collect(out);
        }

        fn wake(&mut self) {
            let mut out = Vec::new();
            self.session.wake(self.now, &mut out);
            self.collect(out);
        }
    }

    #[test]
    fn spawn_returns_a_shared_internet_handle() {
        let (tx, rx) = mpsc::channel();
        let handle = spawn(RELAYS, tx);
        assert_eq!(handle.kind, TransportKind::Internet);
        assert!(handle.shared);
        handle.shutdown();
        drop(rx);
    }

    #[test]
    fn wraps_a_packet_as_a_kind_24747_event_with_our_topic_base64_content_and_a_valid_signature() {
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        harness.broadcast(&[1, 2, 3]);
        let socket = harness.last();
        let events = published(&socket);
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.kind, ROOM_EVENT_KIND);
        assert_eq!(event.tags, [vec!["x".to_string(), harness.keys.topic.clone()]]);
        assert_eq!(event.content, "AQID");
        assert_eq!(event.created_at, NOW / 1000);
        assert!(verify_event(event));
    }

    #[test]
    fn send_to_is_a_broadcast() {
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        harness.broadcast(&[9]);
        assert_eq!(published(&harness.last()).iter().map(|event| event.content.clone()).collect::<Vec<_>>(), ["CQ=="]);
    }

    #[test]
    fn emits_packets_from_other_members() {
        let other = make_key_pair();
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        let event = tagged_event(&other, &harness.keys.topic, "AQID");
        harness.receive(&relay_event(SUB_ID, &event));
        assert_eq!(harness.packets, [(format!("net:{}", other.pubkey), vec![1, 2, 3])]);
    }

    #[test]
    fn drops_content_over_1600_characters_or_that_is_not_base64() {
        let other = make_key_pair();
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        for content in ["A".repeat(1604), "!!!!".to_string(), "AQI".to_string()] {
            let event = tagged_event(&other, &harness.keys.topic, &content);
            harness.receive(&relay_event(SUB_ID, &event));
        }
        assert!(harness.packets.is_empty());
    }

    #[test]
    fn drops_an_event_whose_signature_does_not_hold_up() {
        let other = make_key_pair();
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        let mut event = tagged_event(&other, &harness.keys.topic, "AQID");
        event.content = "AgME".into();
        harness.receive(&relay_event(SUB_ID, &event));
        assert!(harness.packets.is_empty());
    }

    #[test]
    fn uses_a_new_key_pair_on_every_start() {
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        harness.broadcast(&[1]);
        let first = published(&harness.last())[0].pubkey.clone();
        harness.stop();
        harness.start();
        harness.open_last();
        harness.broadcast(&[1]);
        assert_ne!(published(&harness.last())[0].pubkey, first);
    }

    #[test]
    fn stop_waits_for_the_bye_ok() {
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        harness.broadcast(&[7]);
        let bye = published(&harness.last())[0].id.clone();
        harness.stop();
        assert_eq!(harness.session.status(), Link::Unavailable);
        harness.advance(100);
        assert!(harness.closed_at.is_none());
        harness.receive(&ok_reply(&bye, true, ""));
        assert_eq!(harness.closed_at, Some(NOW + 100));
        assert!(harness.last().closed());
    }

    #[test]
    fn stop_waits_at_most_500_ms() {
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        harness.broadcast(&[7]);
        harness.stop();
        harness.advance(499);
        assert!(harness.closed_at.is_none());
        harness.advance(1);
        assert_eq!(harness.closed_at, Some(NOW + 500));
    }

    #[test]
    fn stop_while_connecting_closes_at_once() {
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.broadcast(&[7]);
        harness.stop();
        assert_eq!(harness.closed_at, Some(NOW));
    }

    #[test]
    fn packets_from_a_stopped_session_are_never_emitted() {
        let other = make_key_pair();
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        harness.broadcast(&[7]);
        let old = harness.last();
        harness.stop();
        harness.start();
        harness.open_last();
        old.receive(&relay_event(SUB_ID, &tagged_event(&other, &harness.keys.topic, "AQID")));
        harness.pump();
        assert!(harness.packets.is_empty());
    }

    #[test]
    fn forwards_status_and_link_up_from_the_pool() {
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        harness.broadcast(&[1]);
        let mine = published(&harness.last())[0].clone();
        harness.receive(&relay_event(SUB_ID, &mine));
        harness.stop();
        assert_eq!(harness.statuses, [Link::Connecting, Link::Ok, Link::Unavailable]);
        assert_eq!(harness.link_ups, 1);
    }

    #[test]
    fn wake_reconnects_a_live_relay_and_retry_now_only_reconnects_one_that_is_down() {
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        harness.wake();
        assert_eq!(harness.sockets.borrow().len(), 2);
        harness.open_last();
        harness.retry_now();
        assert_eq!(harness.sockets.borrow().len(), 2);
        harness.drop_last();
        harness.retry_now();
        assert_eq!(harness.sockets.borrow().len(), 3);
    }

    #[test]
    fn stop_after_the_last_event_was_acknowledged_closes_at_once() {
        let mut harness = Net::new(&["wss://a.test"]);
        harness.start();
        harness.open_last();
        harness.broadcast(&[7]);
        let bye = published(&harness.last())[0].id.clone();
        harness.receive(&ok_reply(&bye, true, ""));
        harness.stop();
        assert_eq!(harness.closed_at, Some(NOW));
        assert!(harness.last().closed());
    }

    #[test]
    fn while_draining_a_bye_closes_relays_that_are_not_live_at_once_and_never_reconnects_one() {
        let mut harness = Net::new(&URLS);
        harness.start();
        let (a, b, c) = {
            let sockets = harness.sockets.borrow();
            (sockets[0].clone(), sockets[1].clone(), sockets[2].clone())
        };
        a.open();
        b.open();
        harness.pump();
        b.drop_connection();
        harness.pump();
        harness.broadcast(&[7]);
        harness.stop();
        assert!(c.closed());
        assert!(!a.closed());
        harness.advance(3_000);
        assert_eq!(harness.sockets.borrow().len(), 3);
        assert!(a.closed());
    }

    fn relay_reachable() -> bool {
        let Some((host, port)) = split_url(RELAYS[0]) else { return false };
        let Ok(mut addresses) = (host.as_str(), port).to_socket_addrs() else { return false };
        let Some(address) = addresses.next() else { return false };
        TcpStream::connect_timeout(&address, Duration::from_secs(3)).is_ok()
    }

    #[test]
    fn a_real_relay_carries_our_event() {
        if !relay_reachable() {
            eprintln!("skipped: {} is unreachable, so no relay test runs", RELAYS[0]);
            return;
        }
        let (tx, rx) = mpsc::channel();
        let handle = spawn(RELAYS, tx);
        handle.start(keys());
        let started = std::time::Instant::now();
        let mut linked = false;
        let mut echoed = false;
        while started.elapsed() < Duration::from_secs(30) {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(TransportEvent::LinkUp) => linked = true,
                Ok(TransportEvent::Status(Link::Ok)) => {
                    echoed = true;
                    break;
                }
                Ok(_) => {}
                Err(_) => {}
            }
            if linked {
                handle.broadcast(&[1, 2, 3]);
            } else if started.elapsed() > Duration::from_secs(12) {
                break;
            }
        }
        handle.shutdown();
        if !linked {
            eprintln!("skipped: no relay accepted the connection (a wss handshake needs a rustls crypto provider)");
            return;
        }
        if !echoed {
            eprintln!("skipped: a relay accepted the connection but nothing came back within 30 s");
        }
    }
}
