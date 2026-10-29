//! The tray icon: for each shown agent, its first two windows (5-hour and
//! weekly) as horizontal bars, stacked top to bottom.

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, HICON, ICONINFO};

pub const GREEN: u32 = 0x30D158;
pub const AMBER: u32 = 0xFF9F0A;
pub const RED: u32 = 0xFF453A;
const TRACK: u32 = 0x9A9A9A;
const TRACK_ALPHA: u32 = 0x70;

pub fn severity_color(used: f64) -> u32 {
    if used >= 90.0 {
        RED
    } else if used >= 70.0 {
        AMBER
    } else {
        GREEN
    }
}

/// Straight-alpha ARGB pixels, row-major from the top. `groups` holds the used
/// percentages per agent; `None` draws empty tracks (no data yet).
pub fn render(size: usize, groups: &[Vec<Option<f64>>]) -> Vec<u32> {
    let mut px = vec![0u32; size * size];
    let placeholder = [vec![None, None]];
    let groups: &[Vec<Option<f64>>] = if groups.iter().all(|g| g.is_empty()) { &placeholder } else { groups };
    let groups: Vec<&Vec<Option<f64>>> = groups.iter().filter(|g| !g.is_empty()).collect();

    let bars: usize = groups.iter().map(|g| g.len()).sum();
    let unit = (size / 16).max(1);
    let inner_gap = unit;
    let group_gap = 2 * unit;
    let gaps = (bars - groups.len()) * inner_gap + (groups.len() - 1) * group_gap;
    let margin = unit;
    let avail = size.saturating_sub(2 * margin + gaps);
    let bar_h = (avail / bars).max(1);
    let used_h = bar_h * bars + gaps;
    let mut y = (size - used_h.min(size)) / 2;
    let (x0, x1) = (margin, size - margin);
    let width = x1 - x0;

    for (gi, g) in groups.iter().enumerate() {
        if gi > 0 {
            y += group_gap;
        }
        for (bi, used) in g.iter().enumerate() {
            if bi > 0 {
                y += inner_gap;
            }
            let fill = used.map(|u| ((u / 100.0) * width as f64).round() as usize).unwrap_or(0);
            let fill = if used.is_some_and(|u| u > 0.0) { fill.max(1) } else { fill };
            let color = used.map(severity_color).unwrap_or(TRACK);
            for row in y..(y + bar_h).min(size) {
                for col in x0..x1 {
                    px[row * size + col] = if col - x0 < fill {
                        0xFF00_0000 | color
                    } else {
                        (TRACK_ALPHA << 24) | TRACK
                    };
                }
            }
            y += bar_h;
        }
    }
    px
}

pub fn create(size: usize, pixels: &[u32]) -> Option<HICON> {
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color: HBITMAP = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits.cast::<u32>(), size * size);
        let mask = CreateBitmap(size as i32, size as i32, 1, 1, None);
        let ii = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii).ok();
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_colors(px: &[u32], size: usize, row: usize) -> Vec<u32> {
        px[row * size..(row + 1) * size].to_vec()
    }

    #[test]
    fn two_bars_fill_by_percent() {
        let size = 16;
        let px = render(size, &[vec![Some(50.0), Some(95.0)]]);
        // Find the first painted row: the 5-hour bar, half green.
        let first = (0..size).find(|r| row_colors(&px, size, *r).iter().any(|p| *p != 0)).unwrap();
        let row = row_colors(&px, size, first);
        assert_eq!(row[1], 0xFF00_0000 | GREEN);
        assert_eq!(row[7], 0xFF00_0000 | GREEN);
        assert_eq!(row[8] >> 24, TRACK_ALPHA);
        // The last painted row: the weekly bar, red and nearly full.
        let last = (0..size).rev().find(|r| row_colors(&px, size, *r).iter().any(|p| *p != 0)).unwrap();
        assert_eq!(row_colors(&px, size, last)[13], 0xFF00_0000 | RED);
    }

    #[test]
    fn many_agents_fit() {
        for size in [16, 20, 24, 32] {
            let g = vec![Some(10.0), Some(20.0)];
            let px = render(size, &[g.clone(), g.clone(), g]);
            assert_eq!(px.len(), size * size);
        }
        let px = render(16, &[]);
        assert!(px.iter().any(|p| *p >> 24 == TRACK_ALPHA));
    }
}
