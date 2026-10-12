use std::sync::LazyLock;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, EventMask, PropMode, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent,
    SelectionRequestEvent, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_FROM_PARENT, CURRENT_TIME, NONE};

/// How long a read waits for the owner to answer before giving up.
const READ_TIMEOUT: Duration = Duration::from_millis(400);
const POLL: Duration = Duration::from_millis(5);

enum Request {
    Read(Sender<Option<String>>),
    Write(String),
}

static SERVICE: LazyLock<Sender<Request>> = LazyLock::new(|| {
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("lolping-clipboard".to_string())
        .spawn(move || serve(rx))
        .expect("spawn the clipboard thread");
    tx
});

/// X11 hands out clipboard text only while its owner lives, so the request goes to a thread that owns the selection.
pub fn read_text() -> Option<String> {
    let (tx, rx) = channel();
    SERVICE.send(Request::Read(tx)).ok()?;
    rx.recv_timeout(READ_TIMEOUT + Duration::from_millis(200)).ok().flatten()
}

pub fn write_text(text: &str) -> bool {
    SERVICE.send(Request::Write(text.to_string())).is_ok()
}

struct Atoms {
    clipboard: Atom,
    utf8: Atom,
    targets: Atom,
    text: Atom,
    property: Atom,
}

fn serve(requests: Receiver<Request>) {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots[screen_num].root;
    let Ok(window) = conn.generate_id() else {
        return;
    };
    let aux = CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE);
    let created = conn.create_window(
        0,
        window,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        COPY_FROM_PARENT,
        &aux,
    );
    if created.is_err() {
        return;
    }
    let atoms = Atoms {
        clipboard: intern_atom(&conn, b"CLIPBOARD"),
        utf8: intern_atom(&conn, b"UTF8_STRING"),
        targets: intern_atom(&conn, b"TARGETS"),
        text: intern_atom(&conn, b"TEXT"),
        property: intern_atom(&conn, b"lolPing.clipboard"),
    };
    let mut owned: Option<String> = None;
    let mut pending: Option<(Sender<Option<String>>, Instant)> = None;
    loop {
        while let Ok(request) = requests.try_recv() {
            match request {
                Request::Write(text) => {
                    let _ = conn.set_selection_owner(window, atoms.clipboard, CURRENT_TIME);
                    let _ = conn.flush();
                    owned = Some(text);
                }
                Request::Read(tx) => {
                    if let Some(text) = &owned {
                        let _ = tx.send(Some(text.clone()));
                        continue;
                    }
                    let _ = conn.convert_selection(window, atoms.clipboard, atoms.utf8, atoms.property, CURRENT_TIME);
                    let _ = conn.flush();
                    pending = Some((tx, Instant::now() + READ_TIMEOUT));
                }
            }
        }
        while let Ok(Some(event)) = conn.poll_for_event() {
            handle(&conn, &atoms, &mut owned, &mut pending, event);
        }
        let expired = matches!(&pending, Some((_, deadline)) if Instant::now() >= *deadline);
        if expired {
            if let Some((tx, _)) = pending.take() {
                let _ = tx.send(None);
            }
        }
        std::thread::sleep(POLL);
    }
}

fn handle(
    conn: &RustConnection,
    atoms: &Atoms,
    owned: &mut Option<String>,
    pending: &mut Option<(Sender<Option<String>>, Instant)>,
    event: Event,
) {
    match event {
        Event::SelectionRequest(request) => answer(conn, atoms, owned.as_deref(), request),
        Event::SelectionClear(_) => *owned = None,
        Event::SelectionNotify(notify) => {
            let Some((tx, _)) = pending.take() else {
                return;
            };
            let text = read_property(conn, notify);
            let _ = tx.send(text);
        }
        _ => {}
    }
}

fn answer(conn: &RustConnection, atoms: &Atoms, owned: Option<&str>, request: SelectionRequestEvent) {
    let mut notify = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: request.time,
        requestor: request.requestor,
        selection: request.selection,
        target: request.target,
        property: NONE,
    };
    if let Some(text) = owned {
        if request.property != NONE {
            let string: Atom = AtomEnum::STRING.into();
            if request.target == atoms.targets {
                let list: [u32; 3] = [atoms.targets.into(), atoms.utf8.into(), string.into()];
                let _ = conn.change_property32(PropMode::REPLACE, request.requestor, request.property, AtomEnum::ATOM, &list);
                notify.property = request.property;
            } else if request.target == atoms.utf8 || request.target == string || request.target == atoms.text {
                let _ = conn.change_property8(
                    PropMode::REPLACE,
                    request.requestor,
                    request.property,
                    atoms.utf8,
                    text.as_bytes(),
                );
                notify.property = request.property;
            }
        }
    }
    let _ = conn.send_event(false, request.requestor, EventMask::NO_EVENT, &notify);
    let _ = conn.flush();
}

fn read_property(conn: &RustConnection, notify: SelectionNotifyEvent) -> Option<String> {
    if notify.property == NONE {
        return None;
    }
    let reply = conn
        .get_property(true, notify.requestor, notify.property, AtomEnum::ANY, 0, u32::MAX / 4)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).into_owned())
}

fn intern_atom(conn: &RustConnection, name: &[u8]) -> Atom {
    conn.intern_atom(false, name)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.atom)
        .unwrap_or(NONE)
}
