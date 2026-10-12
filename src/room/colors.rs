/// Name tag colours, picked by index so a peer can never send raw CSS.
pub const TAG_COLORS: [&str; 8] = [
    "#4fc3f7",
    "#ff8a65",
    "#aed581",
    "#ba68c8",
    "#ffd54f",
    "#4db6ac",
    "#f06292",
    "#e0e0e0",
];

pub const TAG_COLOR_COUNT: usize = TAG_COLORS.len();

pub fn is_tag_color(n: i64) -> bool {
    (0..TAG_COLOR_COUNT as i64).contains(&n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_indexed() {
        assert!(is_tag_color(0));
        assert!(is_tag_color(7));
        assert!(!is_tag_color(8));
        assert!(!is_tag_color(-1));
    }
}
