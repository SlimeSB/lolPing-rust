use super::{DisplayMap, Rect};
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

fn monitor_rect(h: HMONITOR) -> Option<(Rect, bool)> {
    let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !unsafe { GetMonitorInfoW(h, &mut info) }.as_bool() {
        return None;
    }
    let r = info.rcMonitor;
    let rect = Rect { x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top };
    Some((rect, info.dwFlags & 1 != 0))
}

pub fn displays() -> Vec<DisplayMap> {
    static OUT: parking_lot::Mutex<Vec<DisplayMap>> = parking_lot::Mutex::new(Vec::new());
    unsafe extern "system" fn cb(h: HMONITOR, _dc: HDC, _r: *mut RECT, _d: LPARAM) -> BOOL {
        if let Some((phys, primary)) = monitor_rect(h) {
            let mut dpi_x = 96u32;
            let mut dpi_y = 96u32;
            let _ = unsafe { GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
            let scale = if dpi_x == 0 { 1.0 } else { f64::from(dpi_x) / 96.0 };
            let dip = Rect {
                x: (f64::from(phys.x) / scale).round() as i32,
                y: (f64::from(phys.y) / scale).round() as i32,
                width: (f64::from(phys.width) / scale).round() as i32,
                height: (f64::from(phys.height) / scale).round() as i32,
            };
            OUT.lock().push(DisplayMap { id: h.0 as u32, dip, phys, scale, primary });
        }
        TRUE
    }
    OUT.lock().clear();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(cb), LPARAM(0));
    }
    OUT.lock().clone()
}
