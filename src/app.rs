//! The UI thread: a hidden message window that owns the tray icon, the
//! refresh timers and the popover.

use crate::config::Config;
use crate::github::GithubState;
use crate::model::{AgentId, AgentState, Status};
use crate::popover::{self, Action, Painter, View};
use crate::worker::{Event, Inbox, Job};
use jiff::Timestamp;
use std::cell::RefCell;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, EndPaint, GetMonitorInfoW,
    HGDIOBJ, InvalidateRect, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, PAINTSTRUCT, SRCCOPY, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, GetDpiForSystem, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};

const WM_TRAY: u32 = WM_APP + 1;
const WM_WORKER: u32 = WM_APP + 2;
const TIMER_REFRESH: usize = 1;
const TIMER_RETRY: usize = 2;
const RETRY_MS: u32 = 60_000;
const MANUAL_THROTTLE: Duration = Duration::from_secs(30);

const MENU_REFRESH: usize = 1;
const MENU_SIGN_IN: usize = 2;
const MENU_SIGN_OUT: usize = 3;
const MENU_FOLDER: usize = 4;
const MENU_PAUSE: usize = 5;
const MENU_QUIT: usize = 6;

struct App {
    hwnd: HWND,
    popover: HWND,
    popover_scale: f32,
    popover_hidden_at: Option<Instant>,
    config: Config,
    states: Vec<AgentState>,
    github: GithubState,
    jobs: Sender<Job>,
    inbox: Inbox,
    refreshing: bool,
    paused: bool,
    retry_armed: bool,
    signing_in: bool,
    last_refresh: Option<Instant>,
    icon: Option<HICON>,
    taskbar_created: u32,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
}

pub fn run() {
    unsafe {
        let instance = GetModuleHandleW(None).expect("module handle");
        let main_class = w!("AgentUsageBar");
        let popover_class = w!("AgentUsageBarPopover");
        for (class, bg) in [(main_class, false), (popover_class, true)] {
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: if bg { LoadCursorW(None, IDC_ARROW).unwrap_or_default() } else { HCURSOR::default() },
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc);
        }
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            main_class,
            w!("agent-usage-bar"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("main window");
        let popover = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            popover_class,
            w!("Agent usage"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("popover window");
        let round = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            popover,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const round).cast(),
            size_of_val(&round) as u32,
        );

        let inbox: Inbox = Default::default();
        let target = hwnd.0 as isize;
        let jobs = crate::worker::spawn(inbox.clone(), move || {
            let _ = PostMessageW(Some(HWND(target as *mut _)), WM_WORKER, WPARAM(0), LPARAM(0));
        });
        let config = Config::load();
        let app = App {
            hwnd,
            popover,
            popover_scale: 1.0,
            popover_hidden_at: None,
            states: AgentId::ALL.iter().map(|a| AgentState::new(*a)).collect(),
            github: GithubState::default(),
            config,
            jobs,
            inbox,
            refreshing: false,
            paused: false,
            retry_armed: false,
            signing_in: false,
            last_refresh: None,
            icon: None,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        with_app(|a| {
            a.add_tray_icon();
            a.arm_refresh_timer();
            a.refresh(None, true);
            // `--show` opens the popover at start (handy after install and for screenshots).
            if std::env::args().any(|a| a == "--show") {
                a.show_popover();
            }
        });

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        with_app(|a| a.remove_tray_icon());
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let handled = with_app(|a| a.handle(hwnd, msg, wparam, lparam)).flatten();
    match handled {
        Some(r) => r,
        None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

impl App {
    fn handle(&mut self, hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        if hwnd == self.popover {
            return self.handle_popover(msg, wparam, lparam);
        }
        match msg {
            WM_TRAY => {
                match (lparam.0 & 0xFFFF) as u32 {
                    NIN_SELECT | WM_LBUTTONUP => self.toggle_popover(),
                    WM_CONTEXTMENU | WM_RBUTTONUP => self.context_menu(),
                    _ => {}
                }
                Some(LRESULT(0))
            }
            WM_WORKER => {
                self.drain_inbox();
                Some(LRESULT(0))
            }
            WM_TIMER => {
                match wparam.0 {
                    TIMER_REFRESH => self.refresh(None, true),
                    TIMER_RETRY
                        if !self.fullscreen() => {
                            self.disarm_retry();
                            self.refresh(None, true);
                        }
                    _ => {}
                }
                Some(LRESULT(0))
            }
            m if m == self.taskbar_created && m != 0 => {
                // Explorer or wdock restarted the notification area.
                self.add_tray_icon();
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn handle_popover(&mut self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_PAINT => {
                self.paint_popover();
                Some(LRESULT(0))
            }
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_ACTIVATE if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE => {
                self.hide_popover();
                Some(LRESULT(0))
            }
            WM_KEYDOWN if wparam.0 as u16 == windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE.0 => {
                self.hide_popover();
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let x = (lparam.0 & 0xFFFF) as i16 as f32 / self.popover_scale;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as f32 / self.popover_scale;
                let layout = popover::layout(&self.view(), None);
                if let Some(action) = layout.hit(x, y) {
                    self.act(action);
                }
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn view(&self) -> View<'_> {
        View {
            states: &self.states,
            github: &self.github,
            config: &self.config,
            refreshing: self.refreshing,
            paused: self.paused,
            signing_in: self.signing_in,
            now: Timestamp::now(),
        }
    }

    fn act(&mut self, action: Action) {
        match action {
            Action::ToggleAgent(id) => {
                self.config.toggle(id);
                self.config.save();
                if self.config.is_shown(id) {
                    self.refresh(Some(vec![id]), false);
                }
                self.changed();
            }
            Action::ToggleGithub => {
                self.config.show_github = !self.config.show_github;
                self.config.save();
                if self.config.show_github {
                    self.refresh(Some(Vec::new()), true);
                }
                self.changed();
            }
            Action::SignInClaude => self.sign_in(),
            Action::OpenGithub => {
                let login = self.github.data.as_ref().map(|c| c.login.clone()).unwrap_or_default();
                crate::shell::open(&format!("https://github.com/{login}"));
            }
            Action::Refresh => self.manual_refresh(),
            Action::CycleInterval => {
                self.config.refresh_minutes = self.config.next_refresh();
                self.config.save();
                self.arm_refresh_timer();
                self.changed();
            }
            Action::Quit => unsafe { PostQuitMessage(0) },
        }
    }

    fn sign_in(&mut self) {
        if !self.signing_in {
            self.signing_in = true;
            let _ = self.jobs.send(Job::SignInClaude);
            self.repaint_popover();
        }
    }

    fn manual_refresh(&mut self) {
        if self.last_refresh.is_some_and(|t| t.elapsed() < MANUAL_THROTTLE) {
            return;
        }
        self.disarm_retry();
        self.refresh(None, true);
    }

    fn fullscreen(&self) -> bool {
        self.config.pause_when_fullscreen && crate::shell::fullscreen_app_in_front()
    }

    /// Queues a refresh of `agents` (default: all shown ones). Network
    /// refreshes are deferred while a full-screen app is in front.
    fn refresh(&mut self, agents: Option<Vec<AgentId>>, with_github: bool) {
        let agents = agents.unwrap_or_else(|| self.config.shown.clone());
        let network = !self.fullscreen();
        self.paused = !network;
        if !network && !self.retry_armed {
            unsafe { SetTimer(Some(self.hwnd), TIMER_RETRY, RETRY_MS, None) };
            self.retry_armed = true;
        }
        self.refreshing = true;
        self.last_refresh = Some(Instant::now());
        let _ = self.jobs.send(Job::Refresh { agents, github: with_github && self.config.show_github, network });
        self.repaint_popover();
    }

    fn disarm_retry(&mut self) {
        if self.retry_armed {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_RETRY);
            }
            self.retry_armed = false;
        }
    }

    fn arm_refresh_timer(&self) {
        unsafe { SetTimer(Some(self.hwnd), TIMER_REFRESH, self.config.refresh_minutes * 60_000, None) };
    }

    fn drain_inbox(&mut self) {
        let events: Vec<Event> = std::mem::take(&mut *self.inbox.lock().unwrap());
        let now = Timestamp::now();
        for e in events {
            match e {
                Event::Agent(id, r) => {
                    if let Some(s) = self.states.iter_mut().find(|s| s.id == id) {
                        s.apply(r, now);
                    }
                }
                Event::Github(r) => self.github.apply(r, now),
                Event::RefreshDone => self.refreshing = false,
                Event::SignInDone(r) => {
                    self.signing_in = false;
                    if r.is_ok() {
                        self.last_refresh = None;
                        self.refresh(Some(vec![AgentId::Claude]), false);
                    }
                }
            }
        }
        self.changed();
    }

    /// State changed: rewrite status.json, the tray icon and the popover.
    fn changed(&mut self) {
        crate::status::write(&self.states, &self.github, &self.config, self.paused);
        self.update_tray_icon();
        self.repaint_popover();
    }

    // ---- tray ----

    fn notify_data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, ..Default::default() }
    }

    fn add_tray_icon(&mut self) {
        self.render_icon();
        let mut nid = self.notify_data();
        nid.uFlags = NIF_ICON | NIF_TIP | NIF_MESSAGE;
        nid.uCallbackMessage = WM_TRAY;
        nid.hIcon = self.icon.unwrap_or_default();
        set_tip(&mut nid, &self.tooltip());
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
    }

    fn update_tray_icon(&mut self) {
        self.render_icon();
        let mut nid = self.notify_data();
        nid.uFlags = NIF_ICON | NIF_TIP;
        nid.hIcon = self.icon.unwrap_or_default();
        set_tip(&mut nid, &self.tooltip());
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
    }

    fn remove_tray_icon(&self) {
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.notify_data());
        }
    }

    fn render_icon(&mut self) {
        let size = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) }.max(16) as usize;
        let groups: Vec<Vec<Option<f64>>> = self
            .states
            .iter()
            .filter(|s| self.config.is_shown(s.id))
            .map(|s| match &s.usage {
                Some(u) if !u.windows.is_empty() => u.windows.iter().take(2).map(|w| Some(w.used_percent)).collect(),
                _ => vec![None, None],
            })
            .collect();
        let pixels = crate::icon::render(size, &groups);
        if let Some(new) = crate::icon::create(size, &pixels)
            && let Some(old) = self.icon.replace(new) {
                unsafe {
                    let _ = DestroyIcon(old);
                }
            }
    }

    fn tooltip(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        for s in self.states.iter().filter(|s| self.config.is_shown(s.id)) {
            let detail = match (&s.usage, s.status) {
                (Some(u), _) => u
                    .windows
                    .iter()
                    .take(2)
                    .map(|w| format!("{} {:.0}%", w.label, w.used_percent))
                    .collect::<Vec<_>>()
                    .join(" · "),
                (None, Status::Pending) => "loading".into(),
                (None, _) => s.message.clone().unwrap_or_default(),
            };
            lines.push(format!("{}: {detail}", s.id.name()));
        }
        if lines.is_empty() {
            lines.push("Agent usage: no agent shown".into());
        }
        if self.paused {
            lines.push("Paused while a full-screen app is in front".into());
        }
        lines.join("\n")
    }

    fn context_menu(&mut self) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else { return };
            let add = |id: usize, text: PCWSTR, flags: MENU_ITEM_FLAGS| {
                let _ = AppendMenuW(menu, MF_STRING | flags, id, text);
            };
            add(MENU_REFRESH, w!("Refresh now"), MF_ENABLED);
            add(MENU_SIGN_IN, w!("Sign in to Claude…"), if self.signing_in { MF_GRAYED } else { MF_ENABLED });
            if crate::providers::claude::has_own_token() {
                add(MENU_SIGN_OUT, w!("Sign out of Claude"), MF_ENABLED);
            }
            add(MENU_FOLDER, w!("Open data folder (status.json)"), MF_ENABLED);
            add(MENU_PAUSE, w!("Pause while full-screen"), if self.config.pause_when_fullscreen { MF_CHECKED } else { MF_UNCHECKED });
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
            add(MENU_QUIT, w!("Quit"), MF_ENABLED);
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let _ = SetForegroundWindow(self.hwnd);
            let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, None, self.hwnd, None);
            let _ = DestroyMenu(menu);
            match cmd.0 as usize {
                MENU_REFRESH => self.manual_refresh(),
                MENU_SIGN_IN => self.sign_in(),
                MENU_SIGN_OUT => {
                    crate::providers::claude::delete_own_token();
                    self.last_refresh = None;
                    self.refresh(Some(vec![AgentId::Claude]), false);
                }
                MENU_FOLDER => {
                    let _ = std::fs::create_dir_all(crate::paths::data_dir());
                    crate::shell::open(&crate::paths::data_dir().to_string_lossy());
                }
                MENU_PAUSE => {
                    self.config.pause_when_fullscreen = !self.config.pause_when_fullscreen;
                    self.config.save();
                }
                MENU_QUIT => PostQuitMessage(0),
                _ => {}
            }
        }
    }

    // ---- popover ----

    fn popover_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.popover).as_bool() }
    }

    fn toggle_popover(&mut self) {
        if self.popover_visible() {
            self.hide_popover();
            return;
        }
        // The click that deactivated (and hid) the popover arrives here too.
        if self.popover_hidden_at.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
            return;
        }
        self.show_popover();
    }

    fn show_popover(&mut self) {
        unsafe {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            self.popover_scale = dx as f32 / 96.0;
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(monitor, &mut info);
            let work: RECT = info.rcWork;
            let w = (popover::WIDTH * self.popover_scale).round() as i32;
            let h = (popover::layout(&self.view(), None).height * self.popover_scale).round() as i32;
            let margin = (8.0 * self.popover_scale) as i32;
            let x = (pt.x - w / 2).clamp(work.left + margin, (work.right - w - margin).max(work.left + margin));
            // Below a top bar (wdock), above a bottom taskbar.
            let y = if pt.y < (work.top + work.bottom) / 2 { work.top + margin } else { work.bottom - h - margin };
            let _ = SetWindowPos(self.popover, Some(HWND_TOPMOST), x, y, w, h, SWP_SHOWWINDOW);
            let _ = SetForegroundWindow(self.popover);
        }
        // Opening the popover is a good moment for fresh numbers.
        if self.last_refresh.is_none_or(|t| t.elapsed() > Duration::from_secs(120)) {
            self.refresh(None, true);
        }
    }

    fn hide_popover(&mut self) {
        if self.popover_visible() {
            unsafe {
                let _ = ShowWindow(self.popover, SW_HIDE);
            }
            self.popover_hidden_at = Some(Instant::now());
        }
    }

    fn repaint_popover(&mut self) {
        if !self.popover_visible() {
            return;
        }
        // Content height changes when agents are toggled or data arrives.
        let h = (popover::layout(&self.view(), None).height * self.popover_scale).round() as i32;
        unsafe {
            let mut r = RECT::default();
            let _ = GetWindowRect(self.popover, &mut r);
            if r.bottom - r.top != h {
                let anchored_bottom = r.top > 200;
                let top = if anchored_bottom { r.bottom - h } else { r.top };
                let _ = SetWindowPos(self.popover, None, r.left, top, r.right - r.left, h, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            let _ = InvalidateRect(Some(self.popover), None, false);
        }
    }

    fn paint_popover(&mut self) {
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(self.popover, &mut ps);
            let mut rc = RECT::default();
            let _ = GetClientRect(self.popover, &mut rc);
            let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
            let mem = CreateCompatibleDC(Some(hdc));
            let bmp = CreateCompatibleBitmap(hdc, w, h);
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            {
                let brush = windows::Win32::Graphics::Gdi::CreateSolidBrush(windows::Win32::Foundation::COLORREF(
                    bgr(popover::background()),
                ));
                windows::Win32::Graphics::Gdi::FillRect(mem, &rc, brush);
                let _ = DeleteObject(brush.into());
                let painter = Painter::new(mem, self.popover_scale);
                popover::layout(&self.view(), Some(&painter));
            }
            let _ = BitBlt(hdc, 0, 0, w, h, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old);
            let _ = DeleteObject(bmp.into());
            let _ = DeleteDC(mem);
            let _ = EndPaint(self.popover, &ps);
        }
    }
}

fn bgr(rgb: u32) -> u32 {
    ((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF)
}

fn set_tip(nid: &mut NOTIFYICONDATAW, text: &str) {
    let wide: Vec<u16> = text.encode_utf16().take(nid.szTip.len() - 1).collect();
    nid.szTip[..wide.len()].copy_from_slice(&wide);
    nid.szTip[wide.len()] = 0;
}
