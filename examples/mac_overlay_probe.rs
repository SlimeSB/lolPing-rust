// Paints a known pattern in one overlay window, so CI can ask the window server and a screenshot whether the Mac overlay path reaches the screen.

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("the overlay probe only runs on macOS");
}

#[cfg(target_os = "macos")]
fn main() {
    probe::main();
}

#[cfg(target_os = "macos")]
mod probe {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use lolping::display;
    use lolping::overlay::window::{self, LayeredWindow};
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_core_foundation::{
        kCFRunLoopCommonModes, CFAbsoluteTimeGetCurrent, CFRunLoop, CFRunLoopTimer, CFRunLoopTimerContext,
    };
    use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Shader, Stroke, Transform};

    const FRAME: Duration = Duration::from_millis(16);
    const FRAMES: usize = 20;
    // Long enough for the screenshot and the window-server query to happen while the window is up.
    const HOLD: Duration = Duration::from_secs(40);

    static TICKS: AtomicUsize = AtomicUsize::new(0);

    pub fn main() {
        let mtm = MainThreadMarker::new().expect("the probe has to run on the main thread");
        let app = NSApplication::sharedApplication(mtm);
        let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        install_tick();
        std::thread::spawn(worker);
        println!("probe: running the app loop");
        app.run();
    }

    // The app pumps window work from a run loop timer on the main thread, so the probe pumps the same way.
    fn install_tick() {
        let context = Box::into_raw(Box::new(CFRunLoopTimerContext {
            version: 0,
            info: std::ptr::null_mut(),
            retain: None,
            release: None,
            copyDescription: None,
        }));
        let timer = unsafe {
            CFRunLoopTimer::new(None, CFAbsoluteTimeGetCurrent() + 0.1, FRAME.as_secs_f64(), 0, 0, Some(on_tick), context)
        };
        let Some(loop_) = CFRunLoop::current() else {
            return;
        };
        if let Some(timer) = &timer {
            unsafe { CFRunLoop::add_timer(&loop_, Some(timer), kCFRunLoopCommonModes) };
        }
    }

    unsafe extern "C-unwind" fn on_tick(_timer: *mut CFRunLoopTimer, _info: *mut std::ffi::c_void) {
        window::pump();
        TICKS.fetch_add(1, Ordering::Relaxed);
    }

    // The main thread keeps the run loop, so a worker owns the window, exactly like the app's overlay thread.
    fn worker() {
        let maps = display::displays();
        println!("probe: displays {maps:?}");
        let Some(map) = maps.iter().find(|map| map.primary).copied() else {
            println!("probe: no primary display");
            std::process::exit(2);
        };
        let window = match LayeredWindow::new(map.phys.x, map.phys.y, map.phys.width, map.phys.height) {
            Ok(window) => window,
            Err(error) => {
                println!("probe: no window: {error}");
                std::process::exit(3);
            }
        };
        let (width, height) = window.size();
        println!("probe: window {width}x{height} display {} dip {:?} phys {:?} scale {}", map.id, map.dip, map.phys, map.scale);
        window.show();
        let mut canvas = match Pixmap::new(width as u32, height as u32) {
            Some(canvas) => canvas,
            None => {
                println!("probe: no canvas for {width}x{height}");
                std::process::exit(4);
            }
        };
        draw(&mut canvas, map.scale as f32);
        for _ in 0..FRAMES {
            if let Err(error) = window.present_rgba(canvas.data()) {
                println!("probe: present failed: {error}");
                std::process::exit(5);
            }
            std::thread::sleep(FRAME);
        }
        println!("probe: painted {FRAMES} frames, holding {:?}", HOLD);
        std::thread::sleep(HOLD);
        let ticks = TICKS.load(Ordering::Relaxed);
        println!("probe: done after {ticks} pump ticks");
        std::process::exit(0);
    }

    // Device pixels: a magenta disc at the centre and a frame at the edges, so the screenshot shows both position and size.
    fn draw(canvas: &mut Pixmap, scale: f32) {
        let paint = Paint { shader: Shader::SolidColor(tiny_skia::Color::from_rgba8(255, 0, 255, 255)), ..Default::default() };
        let (width, height) = (canvas.width() as f32, canvas.height() as f32);
        if let Some(disc) = PathBuilder::from_circle(width / 2.0, height / 2.0, 200.0 * scale) {
            canvas.fill_path(&disc, &paint, FillRule::Winding, Transform::identity(), None);
        }
        if let Some(edge) = tiny_skia::Rect::from_xywh(0.0, 0.0, width, height).map(PathBuilder::from_rect) {
            canvas.stroke_path(&edge, &paint, &Stroke { width: 4.0 * scale, ..Default::default() }, Transform::identity(), None);
        }
    }
}
