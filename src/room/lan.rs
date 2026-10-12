use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_MULTI_SZ,
    REG_VALUE_TYPE,
};

use crate::room::crypto::MAX_PACKET;
use crate::room::flood::{FloodGuard, FloodLimits, DEFAULT_FLOOD_LIMITS};
use crate::room::rate::Clock;
use crate::room::transport::{Link, TransportCommand, TransportEvent, TransportHandle, TransportKind};

pub const LAN_PORT: u16 = 47474;
const INTERFACES_MS: u64 = 5000;
const POLL_MS: u64 = 5;
const MAX_READS_PER_TICK: usize = 64;
/// Longest a stop waits for the sends just queued to leave the socket before closing it.
const DRAIN_MS: u64 = 250;
/// Bigger than MAX_PACKET so an oversized datagram is seen and dropped instead of failing the read.
const READ_BUF: usize = 2048;

#[derive(Clone)]
pub struct LanOptions {
    pub port: u16,
    pub bind_address: Ipv4Addr,
    /// Off in tests: loopback carries no broadcast, and tests shouldn't spray the real network.
    pub broadcast: bool,
    /// Addresses that get every broadcast as unicast too.
    pub seeds: Vec<(Ipv4Addr, u16)>,
    /// Packets per second accepted from one IP before decryption is even tried.
    pub flood_per_second: f64,
    /// Packets per second accepted from all IPs together.
    pub flood_total_per_second: f64,
}

impl Default for LanOptions {
    fn default() -> Self {
        Self {
            port: LAN_PORT,
            bind_address: Ipv4Addr::UNSPECIFIED,
            broadcast: true,
            seeds: Vec::new(),
            flood_per_second: DEFAULT_FLOOD_LIMITS.per_ip,
            flood_total_per_second: DEFAULT_FLOOD_LIMITS.total,
        }
    }
}

pub fn start(events: Sender<TransportEvent>) -> Option<TransportHandle> {
    start_with(events, LanOptions::default())
}

/// Binds the port before returning, so a port already in use reports `None` instead of a dead handle.
pub fn start_with(events: Sender<TransportEvent>, opts: LanOptions) -> Option<TransportHandle> {
    let sock = match bind(&opts) {
        Ok(sock) => sock,
        Err(_) => {
            let _ = events.send(TransportEvent::Status(Link::Unavailable));
            return None;
        }
    };
    let (tx, rx) = mpsc::channel();
    let join = thread::Builder::new().name("room-lan".into()).spawn(move || run(events, opts, sock, rx)).ok()?;
    Some(TransportHandle::new(TransportKind::Lan, false, tx, join))
}

fn bind(opts: &LanOptions) -> std::io::Result<UdpSocket> {
    let sock = UdpSocket::bind(SocketAddrV4::new(opts.bind_address, opts.port))?;
    if opts.broadcast {
        let _ = sock.set_broadcast(true);
    }
    sock.set_read_timeout(Some(Duration::from_millis(POLL_MS)))?;
    sock.set_write_timeout(Some(Duration::from_millis(100)))?;
    Ok(sock)
}

fn run(events: Sender<TransportEvent>, opts: LanOptions, sock: UdpSocket, rx: Receiver<TransportCommand>) {
    let origin = Instant::now();
    let clock: Clock = Arc::new(move || origin.elapsed().as_millis() as u64);
    let limits = FloodLimits {
        per_ip: opts.flood_per_second,
        total: opts.flood_total_per_second,
        max_tracked: DEFAULT_FLOOD_LIMITS.max_tracked,
    };
    let mut flood = FloodGuard::new(clock, limits);
    let mut targets = if opts.broadcast { broadcast_targets() } else { Vec::new() };
    let mut last_refresh = Instant::now();
    let mut last_send: Option<Instant> = None;
    let mut sock = Some(sock);

    send_status(&events, Link::Ok);
    let _ = events.send(TransportEvent::LinkUp);

    loop {
        loop {
            match rx.try_recv() {
                Ok(TransportCommand::Shutdown) => {
                    drain(last_send);
                    return;
                }
                Ok(TransportCommand::Stop) => {
                    drain(last_send);
                    if sock.take().is_some() {
                        send_status(&events, Link::Unavailable);
                    }
                }
                Ok(TransportCommand::Start { .. }) => {
                    if sock.is_none() {
                        open(&opts, &mut sock, &mut targets, &events);
                    }
                }
                Ok(TransportCommand::Wake) | Ok(TransportCommand::Retry) => {
                    drain(last_send);
                    drop(sock.take());
                    open(&opts, &mut sock, &mut targets, &events);
                }
                Ok(TransportCommand::SendTo { peer, bytes }) => {
                    if sock.is_some() {
                        last_send = Some(Instant::now());
                    }
                    send_to(sock.as_ref(), &peer, &bytes);
                }
                Ok(TransportCommand::Broadcast { bytes }) => {
                    if sock.is_some() {
                        last_send = Some(Instant::now());
                    }
                    broadcast(sock.as_ref(), &opts, &targets, &bytes);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if opts.broadcast && last_refresh.elapsed() >= Duration::from_millis(INTERFACES_MS) {
            targets = broadcast_targets();
            last_refresh = Instant::now();
        }
        match sock.as_ref() {
            Some(sock) => read_socket(sock, &mut flood, &events),
            None => thread::sleep(Duration::from_millis(POLL_MS)),
        }
    }
}

/// Node queues sends, so its close drains for up to 250 ms; a stop waits the same long for the last send to leave.
fn drain(last_send: Option<Instant>) {
    if let Some(at) = last_send {
        if let Some(left) = DRAIN_MS.checked_sub(at.elapsed().as_millis() as u64) {
            thread::sleep(Duration::from_millis(left));
        }
    }
}

fn open(opts: &LanOptions, sock: &mut Option<UdpSocket>, targets: &mut Vec<Ipv4Addr>, events: &Sender<TransportEvent>) {
    send_status(events, Link::Connecting);
    match bind(opts) {
        Ok(fresh) => {
            *targets = if opts.broadcast { broadcast_targets() } else { Vec::new() };
            *sock = Some(fresh);
            send_status(events, Link::Ok);
            let _ = events.send(TransportEvent::LinkUp);
        }
        Err(_) => send_status(events, Link::Unavailable),
    }
}

fn read_socket(sock: &UdpSocket, flood: &mut FloodGuard, events: &Sender<TransportEvent>) {
    let mut buf = [0u8; READ_BUF];
    for _ in 0..MAX_READS_PER_TICK {
        match sock.recv_from(&mut buf) {
            Ok((n, from)) if n <= MAX_PACKET => {
                if !flood.allow(&from.ip().to_string()) {
                    continue;
                }
                let peer = format!("lan:{}:{}", from.ip(), from.port());
                if events.send(TransportEvent::Packet { peer, bytes: buf[..n].to_vec() }).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => return,
            Err(_) => return,
        }
    }
}

fn send_to(sock: Option<&UdpSocket>, peer: &str, bytes: &[u8]) {
    let (Some(sock), Some((host, port))) = (sock, parse_peer(peer)) else {
        return;
    };
    let _ = sock.send_to(bytes, SocketAddrV4::new(host, port));
}

fn broadcast(sock: Option<&UdpSocket>, opts: &LanOptions, targets: &[Ipv4Addr], bytes: &[u8]) {
    let Some(sock) = sock else {
        return;
    };
    for host in targets {
        let _ = sock.send_to(bytes, SocketAddrV4::new(*host, opts.port));
    }
    for (host, port) in &opts.seeds {
        let _ = sock.send_to(bytes, SocketAddrV4::new(*host, *port));
    }
}

fn parse_peer(peer: &str) -> Option<(Ipv4Addr, u16)> {
    let (host, port) = peer.strip_prefix("lan:")?.rsplit_once(':')?;
    Some((host.parse().ok()?, port.parse().ok()?))
}

fn send_status(events: &Sender<TransportEvent>, link: Link) {
    let _ = events.send(TransportEvent::Status(link));
}

/// 255.255.255.255 plus the directed broadcast of every IPv4 interface: Windows sends the former out of one interface only.
fn broadcast_targets() -> Vec<Ipv4Addr> {
    let mut out = vec![Ipv4Addr::BROADCAST];
    for (ip, mask) in interface_addresses() {
        let directed = Ipv4Addr::from(u32::from(ip) | !u32::from(mask));
        if !out.contains(&directed) {
            out.push(directed);
        }
    }
    out
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Current IPv4 address and netmask per interface, from the TCP/IP registry key the OS keeps them in.
fn interface_addresses() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    const INTERFACES: &str = r"SYSTEM\CurrentControlSet\Services\Tcpip\Parameters\Interfaces";
    let mut out = Vec::new();
    unsafe {
        let path = wide(INTERFACES);
        let mut root = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(path.as_ptr()), Some(0), KEY_READ, &mut root).0 != 0 {
            return out;
        }
        let mut index = 0u32;
        loop {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            let status = RegEnumKeyExW(root, index, Some(PWSTR(name.as_mut_ptr())), &mut len, None, None, None, None);
            if status.0 != 0 {
                break;
            }
            index += 1;
            let mut sub = HKEY::default();
            if RegOpenKeyExW(root, PCWSTR(name.as_ptr()), Some(0), KEY_READ, &mut sub).0 != 0 {
                continue;
            }
            let ips = multi_sz(sub, "IPAddress");
            let masks = multi_sz(sub, "SubnetMask");
            let _ = RegCloseKey(sub);
            if let (Some(ips), Some(masks)) = (ips, masks) {
                for (ip, mask) in ips.iter().zip(masks.iter()) {
                    if let (Ok(ip), Ok(mask)) = (ip.parse::<Ipv4Addr>(), mask.parse::<Ipv4Addr>()) {
                        if !ip.is_unspecified() && !ip.is_loopback() && !ip.is_broadcast() {
                            out.push((ip, mask));
                        }
                    }
                }
            }
        }
        let _ = RegCloseKey(root);
    }
    out
}

unsafe fn multi_sz(key: HKEY, name: &str) -> Option<Vec<String>> {
    let name = wide(name);
    let mut size = 0u32;
    let mut kind = REG_VALUE_TYPE::default();
    let probe = unsafe { RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, Some(&mut kind), None, Some(&mut size)) };
    if probe.0 != 0 || kind != REG_MULTI_SZ || size == 0 {
        return None;
    }
    let mut buf = vec![0u8; size as usize];
    let read = unsafe { RegQueryValueExW(key, PCWSTR(name.as_ptr()), None, None, Some(buf.as_mut_ptr()), Some(&mut size)) };
    if read.0 != 0 {
        return None;
    }
    let units: Vec<u16> = buf[..size as usize].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(units.split(|u| *u == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::crypto::RoomKeys;
    use parking_lot::Mutex;
    use std::sync::mpsc::channel;

    const A_PORT: u16 = 47_501;
    const B_PORT: u16 = 47_502;
    /// Generous: the whole suite runs at once, and a starved poll thread is not a bug.
    const WAIT: Duration = Duration::from_secs(15);
    const RESEND: Duration = Duration::from_millis(250);
    static SERIAL: Mutex<()> = Mutex::new(());

    fn opts(port: u16, other: u16) -> LanOptions {
        LanOptions {
            port,
            bind_address: Ipv4Addr::LOCALHOST,
            broadcast: false,
            seeds: vec![(Ipv4Addr::LOCALHOST, other)],
            ..LanOptions::default()
        }
    }

    fn keys() -> RoomKeys {
        RoomKeys { msg_key: [7; 32], topic: "00".repeat(32) }
    }

    fn describe(seen: &[TransportEvent]) -> String {
        seen.iter()
            .map(|e| match e {
                TransportEvent::Status(link) => format!("{link:?}"),
                TransportEvent::LinkUp => "LinkUp".to_string(),
                TransportEvent::Packet { bytes, .. } => format!("Packet({})", bytes.len()),
                TransportEvent::PeerGone { peer } => format!("PeerGone({peer})"),
            })
            .collect::<Vec<_>>()
            .join(",")
    }

    fn until(rx: &Receiver<TransportEvent>, mut done: impl FnMut(&[TransportEvent]) -> bool) -> Vec<TransportEvent> {
        let mut seen = Vec::new();
        let end = Instant::now() + WAIT;
        loop {
            while let Ok(ev) = rx.try_recv() {
                seen.push(ev);
            }
            if done(&seen) {
                return seen;
            }
            assert!(Instant::now() < end, "timed out with {} events: [{}]", seen.len(), describe(&seen));
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// UDP is best-effort and the whole suite runs at once, so keep sending until the peer has a packet.
    fn until_packet(rx: &Receiver<TransportEvent>, mut send: impl FnMut(), what: &str) -> Vec<TransportEvent> {
        let mut seen = Vec::new();
        let end = Instant::now() + WAIT;
        let mut sent = Instant::now();
        loop {
            while let Ok(ev) = rx.try_recv() {
                seen.push(ev);
            }
            if !packets(&seen).is_empty() {
                return seen;
            }
            if sent.elapsed() >= RESEND {
                send();
                sent = Instant::now();
            }
            assert!(Instant::now() < end, "{what} never arrived, {} events: [{}]", seen.len(), describe(&seen));
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn status(seen: &[TransportEvent], link: Link) -> bool {
        seen.iter().any(|e| matches!(e, TransportEvent::Status(s) if *s == link))
    }

    fn packets(seen: &[TransportEvent]) -> Vec<&Vec<u8>> {
        seen.iter()
            .filter_map(|e| match e {
                TransportEvent::Packet { bytes, .. } => Some(bytes),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn two_transports_exchange_a_packet_both_ways() {
        let _serial = SERIAL.lock();
        let (a_tx, a_rx) = channel();
        let (b_tx, b_rx) = channel();
        let a = start_with(a_tx, opts(A_PORT, B_PORT)).expect("a binds");
        let b = start_with(b_tx, opts(B_PORT, A_PORT)).expect("b binds");
        until(&a_rx, |s| status(s, Link::Ok));

        let at_b = until_packet(&b_rx, || a.send_to(&format!("lan:127.0.0.1:{B_PORT}"), b"from-a"), "from-a");
        assert_eq!(packets(&at_b)[0], b"from-a");
        assert!(at_b.iter().any(|e| matches!(e, TransportEvent::Packet { peer, .. } if peer == &format!("lan:127.0.0.1:{A_PORT}"))));
        let at_a = until_packet(&a_rx, || b.send_to(&format!("lan:127.0.0.1:{A_PORT}"), b"from-b"), "from-b");
        assert_eq!(packets(&at_a)[0], b"from-b");

        let seeded = until_packet(&b_rx, || a.broadcast(b"seed"), "the seed broadcast");
        assert_eq!(packets(&seeded)[0], b"seed");
        a.shutdown();
        b.shutdown();
    }

    #[test]
    fn caps_junk_from_one_address_before_decryption() {
        let _serial = SERIAL.lock();
        let (b_tx, b_rx) = channel();
        let b = start_with(b_tx, opts(B_PORT, A_PORT)).expect("b binds");
        let junk = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("junk socket");
        for _ in 0..500 {
            let _ = junk.send_to(&[0u8; 64], SocketAddrV4::new(Ipv4Addr::LOCALHOST, B_PORT));
        }
        thread::sleep(Duration::from_millis(200));
        let seen = until(&b_rx, |s| !packets(s).is_empty());
        let through = packets(&seen).len();
        assert!(through > 50 && through <= 130, "through = {through}");
        b.shutdown();
    }

    #[test]
    fn oversized_and_empty_datagrams_are_dropped_without_panicking() {
        let _serial = SERIAL.lock();
        let (b_tx, b_rx) = channel();
        let b = start_with(b_tx, opts(B_PORT, A_PORT)).expect("b binds");
        let junk = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).expect("junk socket");
        let to_b = SocketAddrV4::new(Ipv4Addr::LOCALHOST, B_PORT);
        let _ = junk.send_to(&[0u8; 1500], to_b);
        let _ = junk.send_to(&[0u8; 3000], to_b);
        let _ = junk.send_to(&[], to_b);
        let _ = junk.send_to(b"ok", to_b);

        let seen = until(&b_rx, |s| packets(s).iter().any(|p| p.as_slice() == b"ok"));
        assert!(packets(&seen).iter().all(|p| p.len() <= MAX_PACKET), "an oversized datagram reached the room");
        b.shutdown();
    }

    #[test]
    fn a_taken_port_is_unavailable() {
        let _serial = SERIAL.lock();
        let blocker = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, A_PORT)).expect("blocker binds");
        let (tx, rx) = channel();
        assert!(start_with(tx, opts(A_PORT, B_PORT)).is_none());
        assert!(matches!(rx.try_recv(), Ok(TransportEvent::Status(Link::Unavailable))));
        drop(blocker);
    }

    #[test]
    fn stop_flushes_a_pending_packet_and_frees_the_port() {
        let _serial = SERIAL.lock();
        let (a_tx, a_rx) = channel();
        let (b_tx, b_rx) = channel();
        let a = start_with(a_tx, opts(A_PORT, B_PORT)).expect("a binds");
        let b = start_with(b_tx, opts(B_PORT, A_PORT)).expect("b binds");
        until(&a_rx, |s| status(s, Link::Ok));
        // Three copies: one datagram can be dropped on a machine running the whole suite at once.
        for _ in 0..3 {
            a.send_to(&format!("lan:127.0.0.1:{B_PORT}"), b"bye");
            thread::sleep(Duration::from_millis(30));
        }
        a.shutdown();
        let seen = until(&b_rx, |s| !packets(s).is_empty());
        assert_eq!(packets(&seen)[0], b"bye");
        UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, A_PORT)).expect("port is free again");
        b.shutdown();
    }

    #[test]
    fn stop_then_start_rebinds_on_the_same_handle() {
        let _serial = SERIAL.lock();
        let (a_tx, a_rx) = channel();
        let (b_tx, b_rx) = channel();
        let a = start_with(a_tx, opts(A_PORT, B_PORT)).expect("a binds");
        let b = start_with(b_tx, opts(B_PORT, A_PORT)).expect("b binds");
        until(&a_rx, |s| status(s, Link::Ok));
        a.stop();
        until(&a_rx, |s| status(s, Link::Unavailable));
        a.start(keys());
        until(&a_rx, |s| status(s, Link::Ok));

        let seen = until_packet(&b_rx, || a.send_to(&format!("lan:127.0.0.1:{B_PORT}"), b"restarted"), "restarted");
        assert_eq!(packets(&seen)[0], b"restarted");
        a.shutdown();
        b.shutdown();
    }

    #[test]
    fn wake_reopens_the_socket_and_keeps_working() {
        let _serial = SERIAL.lock();
        let (a_tx, a_rx) = channel();
        let (b_tx, b_rx) = channel();
        let a = start_with(a_tx, opts(A_PORT, B_PORT)).expect("a binds");
        let b = start_with(b_tx, opts(B_PORT, A_PORT)).expect("b binds");
        until(&a_rx, |s| status(s, Link::Ok));
        a.wake();
        until(&a_rx, |s| status(s, Link::Connecting) && status(s, Link::Ok));

        let seen = until_packet(&b_rx, || a.send_to(&format!("lan:127.0.0.1:{B_PORT}"), b"after-wake"), "after-wake");
        assert_eq!(packets(&seen)[0], b"after-wake");
        a.shutdown();
        b.shutdown();
    }
}
