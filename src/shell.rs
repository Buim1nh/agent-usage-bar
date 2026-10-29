//! Small Windows helpers: opening URLs/files and the full-screen check.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::{QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState, ShellExecuteW};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GWL_STYLE, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowThreadProcessId, IsIconic, IsZoomed, SW_SHOWNORMAL, WS_CAPTION,
};
use windows::core::{HSTRING, w};

/// Opens a URL or path with its default handler.
pub fn open(target: &str) {
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(target), None, None, SW_SHOWNORMAL);
    }
}

/// A game, full-screen video or presentation is in front. Same heuristic as
/// wdock: the shell's own notification state first, then a foreground window
/// that exactly covers its monitor without a caption.
pub fn fullscreen_app_in_front() -> bool {
    if let Ok(state) = unsafe { SHQueryUserNotificationState() }
        && (state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_PRESENTATION_MODE || state == QUNS_BUSY)
    {
        return true;
    }
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return false;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(root, Some(&mut pid)) };
    if pid == std::process::id() {
        return false;
    }
    !matches!(class_name(root).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") && covers_monitor(root)
}

fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn covers_monitor(hwnd: HWND) -> bool {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 == WS_CAPTION.0 || IsIconic(hwnd).as_bool() {
            return false;
        }
        let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() {
            return false;
        }
        let mut r = Default::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return false;
        }
        let m = info.rcMonitor;
        let covers = r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
        // Maximized self-framed apps overhang the monitor; full-screen video and
        // borderless games fit it exactly.
        covers && (!IsZoomed(hwnd).as_bool() || r == m)
    }
}
