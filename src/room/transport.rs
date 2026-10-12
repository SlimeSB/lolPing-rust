use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

/// A transport-local address: "lan:<ip>:<port>" or "net:<relay pubkey>".
pub type PeerKey = String;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Link {
    Unavailable,
    Connecting,
    Ok,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TransportKind {
    Lan,
    Internet,
}

/// What a transport reports: sealed packets, a peer that said bye, and its own link state.
pub enum TransportEvent {
    Packet { peer: PeerKey, bytes: Vec<u8> },
    PeerGone { peer: PeerKey },
    Status(Link),
    /// A new link came up: send presence now.
    LinkUp,
}

pub enum TransportCommand {
    Start { keys: crate::room::crypto::RoomKeys },
    /// Closes the current session (draining what was already sent) but keeps the thread alive.
    Stop,
    /// Ends the thread after the same drain; nothing can be started again afterwards.
    Shutdown,
    SendTo { peer: PeerKey, bytes: Vec<u8> },
    Broadcast { bytes: Vec<u8> },
    /// The machine woke up: LAN rebinds its socket, the relays drop their backoff and try again now.
    Wake,
    /// The user asked for the internet again: same as Wake for the relays.
    Retry,
}

/// Moves sealed room packets and nothing else: keys, validation and every decision stay in the room manager.
pub struct TransportHandle {
    pub kind: TransportKind,
    /// One send reaches every member (relays): the manager sends each packet once, never per member.
    pub shared: bool,
    tx: Sender<TransportCommand>,
    join: Option<JoinHandle<()>>,
}

impl TransportHandle {
    pub fn new(kind: TransportKind, shared: bool, tx: Sender<TransportCommand>, join: JoinHandle<()>) -> Self {
        Self { kind, shared, tx, join: Some(join) }
    }

    pub fn start(&self, keys: crate::room::crypto::RoomKeys) {
        let _ = self.tx.send(TransportCommand::Start { keys });
    }

    pub fn send_to(&self, peer: &str, bytes: &[u8]) {
        let _ = self.tx.send(TransportCommand::SendTo { peer: peer.to_string(), bytes: bytes.to_vec() });
    }

    pub fn broadcast(&self, bytes: &[u8]) {
        let _ = self.tx.send(TransportCommand::Broadcast { bytes: bytes.to_vec() });
    }

    /// Asks the transport to stop; its thread flushes what was already sent and closes the session.
    pub fn stop(&self) {
        let _ = self.tx.send(TransportCommand::Stop);
    }

    /// The machine woke up: rebind the LAN socket, poke the relays.
    pub fn wake(&self) {
        let _ = self.tx.send(TransportCommand::Wake);
    }

    /// The user asked for the internet again.
    pub fn retry(&self) {
        let _ = self.tx.send(TransportCommand::Retry);
    }

    /// Stops the transport and waits for the drain to finish.
    pub fn shutdown(mut self) {
        let _ = self.tx.send(TransportCommand::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
