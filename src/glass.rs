//! The popover's glass look: DWM acrylic blur behind the window (the accent
//! API wdock also uses; unlike DWMWA_SYSTEMBACKDROP_TYPE it works on
//! frameless popups) and per-pixel alpha for the content drawn on top.
//!
//! GDI cannot draw alpha, so the content is drawn twice, on black and on
//! white; the difference gives each pixel's coverage (`unmatte`).

use std::ffi::c_void;
use windows::Win32::Foundation::HWND;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::{BOOL, s, w};

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient_color: u32,
    animation_id: u32,
}

#[repr(C)]
struct CompositionAttributeData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

const WCA_ACCENT_POLICY: u32 = 19;
const ACCENT_ENABLE_ACRYLICBLURBEHIND: u32 = 4;

type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionAttributeData) -> BOOL;

/// Acrylic blur behind `hwnd`, tinted with `tint` (0xAARRGGBB).
pub fn set_acrylic(hwnd: HWND, tint: u32) -> bool {
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else { return false };
        let Some(proc) = GetProcAddress(user32, s!("SetWindowCompositionAttribute")) else { return false };
        let set = std::mem::transmute::<unsafe extern "system" fn() -> isize, SetWindowCompositionAttribute>(proc);
        // The API wants 0xAABBGGRR.
        let abgr = (tint & 0xFF00_FF00) | ((tint & 0xFF) << 16) | ((tint >> 16) & 0xFF);
        let mut policy = AccentPolicy { state: ACCENT_ENABLE_ACRYLICBLURBEHIND, flags: 0, gradient_color: abgr, animation_id: 0 };
        let mut data = CompositionAttributeData {
            attribute: WCA_ACCENT_POLICY,
            data: (&raw mut policy).cast(),
            size: size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut data).as_bool()
    }
}

/// Turns the same content drawn on black (`on_black`, overwritten) and on
/// white into premultiplied ARGB: alpha = 255 - (white - black), and the
/// on-black colour is already colour × alpha.
pub fn unmatte(on_black: &mut [u32], on_white: &[u32]) {
    for (b, w) in on_black.iter_mut().zip(on_white) {
        let ch = |v: u32, s: u32| (v >> s) & 0xFF;
        // The channel with the largest difference is the most reliable.
        let diff = [0, 8, 16].iter().map(|s| ch(*w, *s).saturating_sub(ch(*b, *s))).max().unwrap_or(0);
        let a = 255 - diff.min(255);
        let clamp = |s: u32| ch(*b, s).min(a);
        *b = (a << 24) | (clamp(16) << 16) | (clamp(8) << 8) | clamp(0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn unmattes_coverage() {
        // Background, opaque red, and 50% white text.
        let mut black = vec![0x0000_0000, 0x00FF_0000, 0x0080_8080];
        let white = vec![0x00FF_FFFF, 0x00FF_0000, 0x00FF_FFFF];
        super::unmatte(&mut black, &white);
        assert_eq!(black[0] >> 24, 0);
        assert_eq!(black[1], 0xFFFF_0000);
        assert_eq!(black[2], 0x8080_8080);
    }
}
