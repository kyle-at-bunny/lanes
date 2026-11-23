//! The Lanes icon, drawn in code so the tray, the window and the exe all share one design.
//! Also compiled into build.rs (via `#[path]`), so it must only use std.

const BLUE: [u8; 4] = [0x3b, 0x82, 0xf6, 0xff];
const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const ORANGE: [u8; 4] = [0xf9, 0x73, 0x16, 0xff];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

/// Design units: the art is laid out on a 32x32 grid and scaled to `size`.
const GRID: f32 = 32.0;
const CORNER_RADIUS: f32 = 6.0;
const LANE_LEFT_EDGES: [f32; 3] = [8.0, 15.0, 22.0];
const LANE_WIDTH: f32 = 3.0;
const LANE_TOP: f32 = 7.0;
const LANE_BOTTOM: f32 = 25.0;
const DOT_CENTER: (f32, f32) = (25.0, 6.0);
const DOT_RADIUS: f32 = 6.0;

/// RGBA pixels, row-major from the top, for a square icon of `size` pixels.
pub fn rgba(size: u32, attention: bool) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            pixels.extend_from_slice(&pixel(x, y, size, attention));
        }
    }
    pixels
}

/// Three white lanes on a blue rounded square, with an orange dot when something needs attention.
pub fn pixel(x: u32, y: u32, size: u32, attention: bool) -> [u8; 4] {
    let scale = GRID / size as f32;
    let (u, v) = ((x as f32 + 0.5) * scale, (y as f32 + 0.5) * scale);
    if attention && within(u, v, DOT_CENTER, DOT_RADIUS) {
        return ORANGE;
    }
    if !in_rounded_square(u, v) {
        return CLEAR;
    }
    let in_lane = LANE_LEFT_EDGES.iter().any(|&left| (left..left + LANE_WIDTH).contains(&u));
    if in_lane && (LANE_TOP..LANE_BOTTOM).contains(&v) { WHITE } else { BLUE }
}

fn in_rounded_square(u: f32, v: f32) -> bool {
    let nearest = (u.clamp(CORNER_RADIUS, GRID - CORNER_RADIUS), v.clamp(CORNER_RADIUS, GRID - CORNER_RADIUS));
    within(u, v, nearest, CORNER_RADIUS)
}

fn within(u: f32, v: f32, (cx, cy): (f32, f32), radius: f32) -> bool {
    (u - cx).powi(2) + (v - cy).powi(2) <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_corners_lanes_and_attention_dot_at_any_size() {
        assert_eq!(pixel(0, 0, 32, false)[3], 0);
        assert_eq!(pixel(9, 16, 32, false), WHITE);
        assert_eq!(pixel(12, 16, 32, false), BLUE);
        assert_eq!(pixel(25, 6, 32, true), ORANGE);
        assert_ne!(pixel(25, 6, 32, false), ORANGE);
        assert_eq!(pixel(76, 128, 256, false), WHITE, "same design scaled up");
        assert_eq!(pixel(0, 0, 256, false)[3], 0);
        assert_eq!(rgba(16, false).len(), 16 * 16 * 4);
    }
}
