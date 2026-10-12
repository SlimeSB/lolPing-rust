#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }

    fn dist_sq(&self, x: i32, y: i32) -> i64 {
        let dx = (self.x - x).max(0).max(x - (self.x + self.width - 1)) as i64;
        let dy = (self.y - y).max(0).max(y - (self.y + self.height - 1)) as i64;
        dx * dx + dy * dy
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DisplayMap {
    pub id: u32,
    pub dip: Rect,
    pub phys: Rect,
    pub scale: f64,
    pub primary: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalPoint {
    pub display_id: u32,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumberedDisplay {
    pub number: u32,
    pub id: u32,
    pub bounds: Rect,
    pub primary: bool,
}

pub fn display_at_physical(x: i32, y: i32, maps: &[DisplayMap]) -> Option<&DisplayMap> {
    if maps.is_empty() {
        return None;
    }
    if let Some(hit) = maps.iter().find(|m| m.phys.contains(x, y)) {
        return Some(hit);
    }
    maps.iter().reduce(|a, b| if b.phys.dist_sq(x, y) < a.phys.dist_sq(x, y) { b } else { a })
}

/// Physical screen pixels to global DIP coordinates, using the display under the point.
pub fn physical_to_dip(x: i32, y: i32, maps: &[DisplayMap]) -> Option<(DisplayMap, f64, f64)> {
    let d = display_at_physical(x, y, maps)?;
    Some((*d, f64::from(d.dip.x) + f64::from(x - d.phys.x) / d.scale, f64::from(d.dip.y) + f64::from(y - d.phys.y) / d.scale))
}

/// Physical point to pixels inside one overlay, defaulting to the display under the point.
pub fn physical_to_local(x: i32, y: i32, maps: &[DisplayMap], display_id: Option<u32>) -> Option<LocalPoint> {
    let (point_display, dip_x, dip_y) = physical_to_dip(x, y, maps)?;
    let target = match display_id {
        Some(id) => maps.iter().find(|m| m.id == id)?,
        None => &point_display,
    };
    Some(LocalPoint {
        display_id: target.id,
        x: dip_x - f64::from(target.dip.x),
        y: dip_y - f64::from(target.dip.y),
    })
}

/// Pixels inside one overlay back to physical screen pixels: where a room mate's ping lands here.
pub fn local_to_physical(maps: &[DisplayMap], display_id: u32, x: f64, y: f64) -> Option<(i32, i32)> {
    let d = maps.iter().find(|m| m.id == display_id)?;
    Some((d.phys.x + (x * d.scale).round() as i32, d.phys.y + (y * d.scale).round() as i32))
}

/// A display with the number room pings use: #1 is the primary, the rest go left to right, then top to bottom.
pub fn number_displays(displays: &[(u32, Rect, bool)], primary_id: u32) -> Vec<NumberedDisplay> {
    let mut rest: Vec<&(u32, Rect, bool)> = displays.iter().filter(|d| d.0 != primary_id).collect();
    rest.sort_by_key(|d| (d.1.x, d.1.y));
    let primary: Vec<&(u32, Rect, bool)> = displays.iter().filter(|d| d.0 == primary_id).collect();
    primary
        .into_iter()
        .chain(rest)
        .enumerate()
        .map(|(i, d)| NumberedDisplay { number: i as u32 + 1, id: d.0, bounds: d.1, primary: d.0 == primary_id })
        .collect()
}

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// A point inside one display's overlay to that display's number and a fraction of its size.
pub fn to_shared(list: &[NumberedDisplay], display_id: u32, x: f64, y: f64) -> Option<(u32, f64, f64)> {
    let d = list.iter().find(|n| n.id == display_id)?;
    if d.bounds.width <= 0 || d.bounds.height <= 0 {
        return None;
    }
    Some((d.number, clamp01(x / f64::from(d.bounds.width)), clamp01(y / f64::from(d.bounds.height))))
}

/// Where a room ping for display `number` lands here: that display, or the primary without one.
pub fn resolve_target(list: &[NumberedDisplay], number: u32) -> Option<&NumberedDisplay> {
    list.iter().find(|n| n.number == number).or_else(|| list.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect { x, y, width, height }
    }

    fn map(id: u32, phys_x: i32, phys_y: i32, phys_w: i32, phys_h: i32, scale: f64) -> DisplayMap {
        let dip = rect(
            (f64::from(phys_x) / scale).round() as i32,
            (f64::from(phys_y) / scale).round() as i32,
            (f64::from(phys_w) / scale).round() as i32,
            (f64::from(phys_h) / scale).round() as i32,
        );
        DisplayMap { id, dip, phys: rect(phys_x, phys_y, phys_w, phys_h), scale, primary: id == 1 }
    }

    #[test]
    fn numbers_primary_first_then_left_to_right_then_top_to_bottom() {
        let list = number_displays(
            &[(2, rect(1920, 0, 1920, 1080), false), (1, rect(0, 0, 1920, 1080), true), (3, rect(-1920, 200, 1920, 1080), false)],
            1,
        );
        let numbers: Vec<u32> = list.iter().map(|d| d.number).collect();
        let ids: Vec<u32> = list.iter().map(|d| d.id).collect();
        assert_eq!(numbers, vec![1, 2, 3]);
        assert_eq!(ids, vec![1, 3, 2]);
        assert!(list[0].primary);
    }

    #[test]
    fn a_point_outside_every_display_lands_on_the_nearest() {
        let maps = [map(1, 0, 0, 1920, 1080, 1.0), map(2, 1920, 0, 1920, 1080, 1.0)];
        assert_eq!(display_at_physical(2000, 500, &maps).map(|m| m.id), Some(2));
        assert_eq!(display_at_physical(-400, 500, &maps).map(|m| m.id), Some(1));
    }

    #[test]
    fn dip_conversion_uses_the_display_scale() {
        let maps = [map(1, 0, 0, 1536, 864, 1.25)];
        let (_, x, y) = physical_to_dip(240, 120, &maps).expect("point");
        assert!((x - 192.0).abs() < 0.001);
        assert!((y - 96.0).abs() < 0.001);
        let local = physical_to_local(240, 120, &maps, None).expect("local");
        assert_eq!(local.display_id, 1);
        assert!((local.x - 192.0).abs() < 0.001);
    }

    #[test]
    fn a_local_point_can_target_another_display() {
        let maps = [map(1, 0, 0, 1536, 864, 1.25), map(2, 1536, 0, 1536, 864, 1.25)];
        let local = physical_to_local(1536 + 125, 250, &maps, Some(2)).expect("local");
        assert_eq!(local.display_id, 2);
        assert!((local.x - 100.0).abs() < 0.001);
        assert!((local.y - 200.0).abs() < 0.001);
        assert_eq!(local_to_physical(&maps, 2, local.x, local.y), Some((1536 + 125, 250)));
    }

    #[test]
    fn shared_points_are_clamped_fractions() {
        let list = number_displays(&[(1, rect(0, 0, 1000, 500), true)], 1);
        assert_eq!(to_shared(&list, 1, 250.0, 125.0), Some((1, 0.25, 0.25)));
        assert_eq!(to_shared(&list, 1, -50.0, 900.0), Some((1, 0.0, 1.0)));
        assert_eq!(to_shared(&list, 9, 1.0, 1.0), None);
    }

    #[test]
    fn a_missing_display_falls_back_to_the_primary() {
        let list = number_displays(&[(1, rect(0, 0, 1000, 500), true), (2, rect(1000, 0, 1000, 500), false)], 1);
        assert_eq!(resolve_target(&list, 2).map(|d| d.id), Some(2));
        assert_eq!(resolve_target(&list, 7).map(|d| d.id), Some(1));
    }}
