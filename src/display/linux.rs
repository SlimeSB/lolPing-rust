use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

use super::{DisplayMap, Rect};

const DEFAULT_DPI: f64 = 96.0;

/// Monitors through RandR, or the whole screen when the server has no RandR to ask.
pub fn displays() -> Vec<DisplayMap> {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return Vec::new();
    };
    let screen = &conn.setup().roots[screen_num];
    let root = screen.root;
    let scale = xft_dpi(&conn, root) / DEFAULT_DPI;
    let whole = Rect {
        x: 0,
        y: 0,
        width: i32::from(screen.width_in_pixels),
        height: i32::from(screen.height_in_pixels),
    };
    let Some((resources, primary)) = randr(&conn, root) else {
        return vec![map(0, whole, true, scale)];
    };
    let mut out = Vec::new();
    for crtc in resources.crtcs {
        let Some(info) = conn
            .randr_get_crtc_info(crtc, resources.config_timestamp)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        else {
            continue;
        };
        if info.width == 0 || info.height == 0 {
            continue;
        }
        let rect = Rect {
            x: i32::from(info.x),
            y: i32::from(info.y),
            width: i32::from(info.width),
            height: i32::from(info.height),
        };
        out.push(map(u32::from(crtc), rect, info.outputs.contains(&primary), scale));
    }
    if out.is_empty() {
        vec![map(0, whole, true, scale)]
    } else {
        if !out.iter().any(|display| display.primary) {
            if let Some(first) = out.first_mut() {
                first.primary = true;
            }
        }
        out
    }
}

fn randr(
    conn: &RustConnection,
    root: Window,
) -> Option<(x11rb::protocol::randr::GetScreenResourcesCurrentReply, x11rb::protocol::randr::Output)> {
    let resources = conn.randr_get_screen_resources_current(root).ok()?.reply().ok()?;
    let primary = conn.randr_get_output_primary(root).ok()?.reply().ok()?.output;
    Some((resources, primary))
}

fn map(id: u32, phys: Rect, primary: bool, scale: f64) -> DisplayMap {
    DisplayMap {
        id,
        dip: Rect {
            x: (f64::from(phys.x) / scale).round() as i32,
            y: (f64::from(phys.y) / scale).round() as i32,
            width: (f64::from(phys.width) / scale).round() as i32,
            height: (f64::from(phys.height) / scale).round() as i32,
        },
        phys,
        scale,
        primary,
    }
}

/// X11 has one DPI for the whole session, and Xft.dpi is where the desktops keep it.
fn xft_dpi(conn: &RustConnection, root: Window) -> f64 {
    let Some(atom) = conn
        .intern_atom(false, b"Xft.dpi")
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return DEFAULT_DPI;
    };
    let Some(reply) = conn
        .get_property(false, root, atom.atom, AtomEnum::STRING, 0, 64)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return DEFAULT_DPI;
    };
    String::from_utf8_lossy(&reply.value)
        .lines()
        .find_map(|line| line.strip_prefix("Xft.dpi:"))
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|dpi| *dpi > 0.0)
        .unwrap_or(DEFAULT_DPI)
}
