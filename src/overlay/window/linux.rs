use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::shm::{ConnectionExt as _, Seg};
use x11rb::protocol::xproto::{
    AtomEnum, ClipOrdering, Colormap, ColormapAlloc, ConfigureWindowAux, ConnectionExt as _, CreateGCAux, CreateWindowAux,
    EventMask, Gcontext, ImageFormat, PropMode, Screen, StackMode, VisualClass, Visualid, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::NONE;

/// A click-through, always-on-top window whose pixels carry per-pixel alpha.
pub struct LayeredWindow {
    conn: RustConnection,
    window: Window,
    gc: Gcontext,
    colormap: Colormap,
    width: i32,
    height: i32,
    depth: u8,
    shm: Option<Segment>,
}

struct Segment {
    segment: Seg,
    address: *mut u8,
}

impl LayeredWindow {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Result<Self, String> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|error| error.to_string())?;
        let screen = conn.setup().roots[screen_num].clone();
        let (depth, visual) = argb_visual(&screen).ok_or("no 32-bit ARGB visual")?;
        let colormap = conn.generate_id().map_err(|error| error.to_string())?;
        conn.create_colormap(ColormapAlloc::NONE, colormap, screen.root, visual)
            .map_err(|error| error.to_string())?;
        let window = conn.generate_id().map_err(|error| error.to_string())?;
        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .colormap(colormap)
            .override_redirect(1)
            .event_mask(EventMask::NO_EVENT);
        conn.create_window(
            depth,
            window,
            screen.root,
            x as i16,
            y as i16,
            width as u16,
            height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &aux,
        )
        .map_err(|error| error.to_string())?;
        // No input region at all: the wheel is painted over the screen, never in the pointer's way.
        conn.shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, window, 0, 0, &[])
            .map_err(|error| error.to_string())?;
        let state = intern_atom(&conn, b"_NET_WM_STATE");
        let above = intern_atom(&conn, b"_NET_WM_STATE_ABOVE");
        let _ = conn.change_property32(PropMode::REPLACE, window, state, AtomEnum::ATOM, &[above.into()]);
        let gc = conn.generate_id().map_err(|error| error.to_string())?;
        conn.create_gc(gc, window, &CreateGCAux::new().graphics_exposures(0))
            .map_err(|error| error.to_string())?;
        conn.flush().map_err(|error| error.to_string())?;
        let shm = attach(&conn, width, height);
        Ok(Self {
            conn,
            window,
            gc,
            colormap,
            width,
            height,
            depth,
            shm,
        })
    }

    pub fn size(&self) -> (i32, i32) {
        (self.width, self.height)
    }

    /// `rgba` is premultiplied RGBA (what a tiny-skia pixmap holds), `width * height * 4` bytes.
    pub fn present_rgba(&self, rgba: &[u8]) -> Result<(), String> {
        let count = (self.width * self.height) as usize;
        if rgba.len() < count * 4 {
            return Err(format!("short buffer: {} < {}", rgba.len(), count * 4));
        }
        let (width, height) = (self.width as u16, self.height as u16);
        match &self.shm {
            Some(segment) => {
                let dst = unsafe { std::slice::from_raw_parts_mut(segment.address, count * 4) };
                pack(rgba, dst);
                self.conn
                    .shm_put_image(
                        self.window,
                        self.gc,
                        width,
                        height,
                        0,
                        0,
                        width,
                        height,
                        0,
                        0,
                        self.depth,
                        u8::from(ImageFormat::Z_PIXMAP),
                        false,
                        segment.segment,
                        0,
                    )
                    .map_err(|error| error.to_string())?;
            }
            None => {
                // Only a server without MIT-SHM gets here, so the copy is worth its cost.
                let mut packed = vec![0u8; count * 4];
                pack(rgba, &mut packed);
                self.conn
                    .put_image(
                        ImageFormat::Z_PIXMAP,
                        self.window,
                        self.gc,
                        width,
                        height,
                        0,
                        0,
                        0,
                        self.depth,
                        &packed,
                    )
                    .map_err(|error| error.to_string())?;
            }
        }
        // A window manager stacks what it manages above unmanaged windows, so ask again every frame.
        let _ = self
            .conn
            .configure_window(self.window, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        self.conn.flush().map_err(|error| error.to_string())
    }

    pub fn show(&self) {
        let _ = self.conn.map_window(self.window);
        let _ = self.conn.flush();
    }
}

impl Drop for LayeredWindow {
    fn drop(&mut self) {
        if let Some(segment) = self.shm.take() {
            let _ = self.conn.shm_detach(segment.segment);
            unsafe { libc::shmdt(segment.address.cast()) };
        }
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.free_gc(self.gc);
        let _ = self.conn.free_colormap(self.colormap);
        let _ = self.conn.flush();
    }
}

/// The server wants the pixel word of a 32-bit visual, and little-endian puts its blue byte first.
fn pack(rgba: &[u8], dst: &mut [u8]) {
    for (index, pixel) in dst.chunks_exact_mut(4).enumerate() {
        let source = index * 4;
        pixel[0] = rgba[source + 2];
        pixel[1] = rgba[source + 1];
        pixel[2] = rgba[source];
        pixel[3] = rgba[source + 3];
    }
}

fn argb_visual(screen: &Screen) -> Option<(u8, Visualid)> {
    for depth in &screen.allowed_depths {
        if depth.depth != 32 {
            continue;
        }
        for visual in &depth.visuals {
            if visual.class == VisualClass::TRUE_COLOR
                && visual.red_mask == 0xff0000
                && visual.green_mask == 0xff00
                && visual.blue_mask == 0xff
            {
                return Some((depth.depth, visual.visual_id));
            }
        }
    }
    None
}

fn attach(conn: &RustConnection, width: i32, height: i32) -> Option<Segment> {
    let size = (width.max(1) as usize) * (height.max(1) as usize) * 4;
    let id = unsafe { libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600) };
    if id < 0 {
        return None;
    }
    // Marked for deletion at once: the kernel reclaims it when the last mapping goes, crash included.
    unsafe { libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut()) };
    let address = unsafe { libc::shmat(id, std::ptr::null(), 0) };
    if address as isize == -1 {
        return None;
    }
    let Some(segment) = conn.generate_id().ok() else {
        unsafe { libc::shmdt(address) };
        return None;
    };
    if conn.shm_attach(segment, id as u32, false).is_err() {
        unsafe { libc::shmdt(address) };
        return None;
    }
    Some(Segment {
        segment,
        address: address as *mut u8,
    })
}

fn intern_atom(conn: &RustConnection, name: &[u8]) -> x11rb::protocol::xproto::Atom {
    conn.intern_atom(false, name)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.atom)
        .unwrap_or(NONE)
}
