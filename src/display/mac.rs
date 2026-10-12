use objc2_core_foundation::CGRect;
use objc2_core_graphics::{
    CGDirectDisplayID, CGDisplayBounds, CGDisplayPixelsWide, CGGetActiveDisplayList, CGMainDisplayID,
};

use crate::display::{DisplayMap, Rect};

/// Monitors through CoreGraphics: bounds are points from the main display's top left, pixels come from the backing store.
pub fn displays() -> Vec<DisplayMap> {
    let mut ids = [0 as CGDirectDisplayID; 16];
    let mut count = 0u32;
    let error = unsafe { CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) };
    if error.0 != 0 {
        return Vec::new();
    }
    let primary = CGMainDisplayID();
    let found = (count as usize).min(ids.len());
    ids[..found].iter().map(|id| map_for(*id, primary)).collect()
}

fn map_for(id: CGDirectDisplayID, primary: CGDirectDisplayID) -> DisplayMap {
    let bounds: CGRect = CGDisplayBounds(id);
    let dip = Rect {
        x: bounds.origin.x.round() as i32,
        y: bounds.origin.y.round() as i32,
        width: bounds.size.width.round() as i32,
        height: bounds.size.height.round() as i32,
    };
    let pixels = CGDisplayPixelsWide(id) as i32;
    let scale = if dip.width > 0 { f64::from(pixels) / f64::from(dip.width) } else { 1.0 };
    let phys = Rect {
        x: (f64::from(dip.x) * scale).round() as i32,
        y: (f64::from(dip.y) * scale).round() as i32,
        width: (f64::from(dip.width) * scale).round() as i32,
        height: (f64::from(dip.height) * scale).round() as i32,
    };
    DisplayMap { id, dip, phys, scale, primary: id == primary }
}
