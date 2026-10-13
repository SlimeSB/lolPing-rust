use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};

use objc2::rc::Retained;
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSBackingStoreType, NSBitmapImageRep, NSColor, NSImage, NSImageView, NSScreenSaverWindowLevel, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_core_foundation::{CGPoint, CGSize};
use objc2_core_graphics::{CGDisplayBounds, CGMainDisplayID};
use objc2_foundation::{NSRect, NSSize};

use crate::display::{self, DisplayMap};

type Job = Box<dyn FnOnce() + Send>;

/// AppKit only runs on the main thread, so the overlay thread leaves its window work here.
static JOBS: Mutex<Vec<Job>> = Mutex::new(Vec::new());
static NEXT_ID: AtomicUsize = AtomicUsize::new(1);

thread_local! {
    static LAYERS: RefCell<Vec<Layer>> = const { RefCell::new(Vec::new()) };
}

struct Layer {
    id: usize,
    window: Retained<NSWindow>,
    view: Retained<NSImageView>,
    image: Retained<NSImage>,
    dirty: Arc<AtomicBool>,
}

struct Pixels(*mut u8);

unsafe impl Send for Pixels {}

/// Runs the queued window work and repaints what the overlay thread drew: the main thread calls this every frame.
pub fn pump() {
    let jobs: Vec<Job> = match JOBS.lock() {
        Ok(mut jobs) => std::mem::take(&mut *jobs),
        Err(_) => return,
    };
    for job in jobs {
        job();
    }
    LAYERS.with(|layers| {
        for layer in layers.borrow().iter() {
            if layer.dirty.swap(false, Ordering::Relaxed) {
                // The pixels go straight into the rep, so the image must be told to drop its cached copy of them.
                layer.image.recache();
                let view: &NSView = &layer.view;
                view.setNeedsDisplay(true);
            }
        }
    });
}

/// A click-through, always-on-top window whose pixels carry per-pixel alpha.
pub struct LayeredWindow {
    id: usize,
    pixels: Pixels,
    bytes_per_row: usize,
    width: i32,
    height: i32,
    dirty: Arc<AtomicBool>,
}

impl LayeredWindow {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Result<Self, String> {
        let (tx, rx) = channel();
        {
            let mut jobs = JOBS.lock().map_err(|_| "the window queue is poisoned".to_string())?;
            jobs.push(Box::new(move || {
                let _ = tx.send(create(x, y, width, height));
            }));
        }
        rx.recv().map_err(|_| "the main thread never ran the overlay window job".to_string())?
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
        let row_bytes = self.width as usize * 4;
        if self.bytes_per_row == row_bytes {
            unsafe { std::ptr::copy_nonoverlapping(rgba.as_ptr(), self.pixels.0, count * 4) };
        } else {
            for row in 0..self.height as usize {
                unsafe {
                    let dst = self.pixels.0.add(row * self.bytes_per_row);
                    std::ptr::copy_nonoverlapping(rgba.as_ptr().add(row * row_bytes), dst, row_bytes);
                }
            }
        }
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn show(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }
}

impl Drop for LayeredWindow {
    fn drop(&mut self) {
        let id = self.id;
        if let Ok(mut jobs) = JOBS.lock() {
            jobs.push(Box::new(move || close(id)));
        }
    }
}

fn close(id: usize) {
    LAYERS.with(|layers| {
        let mut layers = layers.borrow_mut();
        let Some(index) = layers.iter().position(|layer| layer.id == id) else {
            return;
        };
        let layer = layers.remove(index);
        unsafe { layer.window.setReleasedWhenClosed(false) };
        layer.window.close();
    });
}

fn create(x: i32, y: i32, width: i32, height: i32) -> Result<LayeredWindow, String> {
    let mtm = MainThreadMarker::new().ok_or("the overlay window has to be built on the main thread")?;
    let maps = display::displays();
    let map = maps
        .iter()
        .find(|m| m.phys.x == x && m.phys.y == y && m.phys.width == width && m.phys.height == height)
        .ok_or_else(|| format!("no display at {x},{y} of {width}x{height}"))?;
    let size = NSSize { width: f64::from(map.dip.width), height: f64::from(map.dip.height) };
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width as isize,
            height as isize,
            8,
            4,
            true,
            false,
            objc2_app_kit::NSDeviceRGBColorSpace,
            (width * 4) as isize,
            32,
        )
    }
    .ok_or("no bitmap image rep")?;
    rep.setSize(size);
    let image = NSImage::initWithSize(NSImage::alloc(), size);
    image.addRepresentation(&rep);
    let view = NSImageView::initWithFrame(NSImageView::alloc(mtm), NSRect { origin: CGPoint { x: 0.0, y: 0.0 }, size });
    view.setImage(Some(&image));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame(map),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    window.setIgnoresMouseEvents(true);
    window.setLevel(NSScreenSaverWindowLevel);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
    window.setContentView(Some(&view));
    window.orderFrontRegardless();

    let dirty = Arc::new(AtomicBool::new(true));
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let layer = Layer { id, window, view, image, dirty: Arc::clone(&dirty) };
    LAYERS.with(|layers| layers.borrow_mut().push(layer));
    Ok(LayeredWindow {
        id,
        pixels: Pixels(rep.bitmapData()),
        bytes_per_row: rep.bytesPerRow() as usize,
        width,
        height,
        dirty,
    })
}

fn frame(map: &DisplayMap) -> NSRect {
    let primary_height = CGDisplayBounds(CGMainDisplayID()).size.height;
    NSRect {
        origin: CGPoint {
            x: f64::from(map.dip.x),
            y: primary_height - f64::from(map.dip.y) - f64::from(map.dip.height),
        },
        size: CGSize { width: f64::from(map.dip.width), height: f64::from(map.dip.height) },
    }
}
