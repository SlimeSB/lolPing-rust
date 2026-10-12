pub struct WheelMetrics {
    pub inner_r: f32,
    pub outer_r: f32,
    pub ext_r: f32,
    pub icon_r: f32,
    pub icon_size: f32,
}

pub const WHEEL: WheelMetrics =
    WheelMetrics { inner_r: 86.0, outer_r: 188.0, ext_r: 288.0, icon_r: 137.0, icon_size: 48.0 };

/// Slice under an offset from the wheel centre: None inside the dead zone, else 0..8 with 0 at the top going clockwise.
pub fn pick_slice(dx: f32, dy: f32, dead_zone: f32) -> Option<usize> {
    if dx.hypot(dy) < dead_zone {
        return None;
    }
    let deg = (dy.atan2(dx).to_degrees() + 90.0 + 22.5 + 360.0) % 360.0;
    Some((deg / 45.0) as usize % 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dead_zone_has_no_slice() {
        assert_eq!(pick_slice(10.0, 10.0, 86.0), None);
    }

    #[test]
    fn slices_start_at_the_top_and_run_clockwise() {
        assert_eq!(pick_slice(0.0, -200.0, 86.0), Some(0));
        assert_eq!(pick_slice(200.0, 0.0, 86.0), Some(2));
        assert_eq!(pick_slice(0.0, 200.0, 86.0), Some(4));
        assert_eq!(pick_slice(-200.0, 0.0, 86.0), Some(6));
    }

    #[test]
    fn a_slice_edge_belongs_to_one_side_only() {
        let edge = pick_slice(-45.0f32.to_radians().tan() * 100.0, -100.0, 86.0);
        assert!(edge.is_some());
    }
}
