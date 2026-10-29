//! The popover ("big numbers" design): a block per shown agent with its main
//! window as a large percentage and the other windows as thin bars, the
//! shared GitHub contribution graph, and a row of chips that turn agents and
//! GitHub on or off. Refresh and the interval sit in the header.
//! `layout` is used both to size/hit-test (no device context) and to paint.

use crate::config::Config;
use crate::github::GithubState;
use crate::icon::severity_color;
use crate::model::{AgentId, AgentState, Status, Window, format_until};
use jiff::Timestamp;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
    DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
    FF_DONTCARE, FW_NORMAL, FW_SEMIBOLD, FillRect, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, PS_SOLID, RoundRect,
    GetStockObject, NULL_BRUSH, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::core::{PCWSTR, w};

pub const WIDTH: f32 = 340.0;
const PAD: f32 = 16.0;
const INNER: f32 = WIDTH - 2.0 * PAD;

/// Acrylic tint behind the content, 0xAARRGGBB: dark glass.
pub const GLASS_TINT: u32 = 0x8C1C1C1E;
/// What "no fill" draws as; the glass shows through it.
const BG: u32 = 0x000000;
const TEXT: u32 = 0xF2F2F7;
const DIM: u32 = 0x8E8E93;
const TRACK: u32 = 0x48484C;
const DIVIDER: u32 = 0x55555A;
const CHIP_ON: u32 = 0x3A3A3E;
const CHIP_ON_BORDER: u32 = 0x7C7C80;
const CHIP_OFF_BORDER: u32 = 0x66666B;
const LINK: u32 = 0x64A8FF;
/// GitHub's dark-theme contribution shades, level 0..=4.
const GH_LEVELS: [u32; 5] = [0x2D333B, 0x0E4429, 0x006D32, 0x26A641, 0x39D353];
/// Segoe Fluent Icons "Refresh".
const ICON_REFRESH: &str = "\u{E72C}";

fn agent_color(id: AgentId) -> u32 {
    match id {
        AgentId::Claude => 0xD97757,
        AgentId::Codex => 0x10A37F,
        AgentId::Antigravity => 0x4F8CFF,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleAgent(AgentId),
    ToggleGithub,
    SignInClaude,
    OpenGithub,
    Refresh,
    CycleInterval,
}

pub struct View<'a> {
    pub states: &'a [AgentState],
    pub github: &'a GithubState,
    pub config: &'a Config,
    pub refreshing: bool,
    pub paused: bool,
    pub signing_in: bool,
    pub now: Timestamp,
}

/// Rectangles in DIPs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

pub struct Layout {
    pub height: f32,
    pub hits: Vec<(Rect, Action)>,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Action> {
        self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a)
    }
}

#[derive(Clone, Copy)]
enum Font {
    Title,
    Big,
    Body,
    Small,
    Icon,
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

pub struct Painter {
    hdc: HDC,
    scale: f32,
    fonts: [HFONT; 5],
}

fn colorref(rgb: u32) -> COLORREF {
    COLORREF(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
}

impl Painter {
    pub fn new(hdc: HDC, scale: f32) -> Painter {
        let font = |size: f32, weight: u32, face: PCWSTR| unsafe {
            CreateFontW(
                -(size * scale).round() as i32,
                0,
                0,
                0,
                weight as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                ANTIALIASED_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                face,
            )
        };
        unsafe { SetBkMode(hdc, TRANSPARENT) };
        let ui = w!("Segoe UI Variable Text");
        Painter {
            hdc,
            scale,
            fonts: [
                font(13.5, FW_SEMIBOLD.0, ui),
                font(28.0, FW_SEMIBOLD.0, w!("Segoe UI Variable Display")),
                font(12.5, FW_NORMAL.0, ui),
                font(11.5, FW_NORMAL.0, ui),
                font(12.0, FW_NORMAL.0, w!("Segoe Fluent Icons")),
            ],
        }
    }

    fn px(&self, v: f32) -> i32 {
        (v * self.scale).round() as i32
    }

    fn rect(&self, r: Rect) -> RECT {
        RECT { left: self.px(r.x), top: self.px(r.y), right: self.px(r.x + r.w), bottom: self.px(r.y + r.h) }
    }

    fn fill(&self, r: Rect, rgb: u32) {
        unsafe {
            let b = CreateSolidBrush(colorref(rgb));
            FillRect(self.hdc, &self.rect(r), b);
            let _ = DeleteObject(b.into());
        }
    }

    /// A rounded rectangle filled with `rgb` and outlined with `border`.
    fn round(&self, r: Rect, radius: f32, rgb: u32, border: u32) {
        let rc = self.rect(r);
        unsafe {
            let b = CreateSolidBrush(colorref(rgb));
            let p = CreatePen(PS_SOLID, 1, colorref(border));
            // BG means "no fill": leave the glass visible inside the outline.
            let fill = if rgb == BG { GetStockObject(NULL_BRUSH) } else { HGDIOBJ(b.0) };
            let ob = SelectObject(self.hdc, fill);
            let op = SelectObject(self.hdc, HGDIOBJ(p.0));
            let d = self.px(radius * 2.0).max(1);
            let _ = RoundRect(self.hdc, rc.left, rc.top, rc.right, rc.bottom, d, d);
            SelectObject(self.hdc, ob);
            SelectObject(self.hdc, op);
            let _ = DeleteObject(b.into());
            let _ = DeleteObject(p.into());
        }
    }

    fn solid(&self, r: Rect, radius: f32, rgb: u32) {
        self.round(r, radius, rgb, rgb);
    }

    /// Width of `s` in DIPs.
    fn measure(&self, s: &str, font: Font) -> f32 {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let mut size = windows::Win32::Foundation::SIZE::default();
        unsafe {
            let old = SelectObject(self.hdc, HGDIOBJ(self.fonts[font as usize].0));
            let _ = windows::Win32::Graphics::Gdi::GetTextExtentPoint32W(self.hdc, &wide, &mut size);
            SelectObject(self.hdc, old);
        }
        size.cx as f32 / self.scale
    }

    fn text(&self, r: Rect, s: &str, font: Font, rgb: u32, align: Align) {
        let mut rc = self.rect(r);
        let mut wide: Vec<u16> = s.encode_utf16().collect();
        let flags = DT_SINGLELINE
            | DT_VCENTER
            | DT_NOPREFIX
            | DT_END_ELLIPSIS
            | match align {
                Align::Left => DT_LEFT,
                Align::Center => DT_CENTER,
                Align::Right => DT_RIGHT,
            };
        unsafe {
            let old = SelectObject(self.hdc, HGDIOBJ(self.fonts[font as usize].0));
            SetTextColor(self.hdc, colorref(rgb));
            DrawTextW(self.hdc, &mut wide, &mut rc, flags);
            SelectObject(self.hdc, old);
        }
    }
}

impl Drop for Painter {
    fn drop(&mut self) {
        for f in self.fonts {
            unsafe {
                let _ = DeleteObject(f.into());
            }
        }
    }
}

/// Approximate text width in DIPs, used for layout without a device
/// context so painting and hit-testing agree.
fn text_width(s: &str, size: f32) -> f32 {
    s.chars().map(|c| if c.is_ascii_uppercase() { 0.68 } else if c == ' ' { 0.3 } else { 0.56 }).sum::<f32>() * size
}

fn ago(t: Timestamp, now: Timestamp) -> String {
    let mins = (now.as_second() - t.as_second()) / 60;
    match mins {
        i64::MIN..=0 => "just now".into(),
        1..=59 => format!("{mins}m ago"),
        _ => format!("{}h ago", mins / 60),
    }
}

/// "of 5-hour limit" for the big number's caption.
fn caption(w: &Window) -> String {
    match w.label.as_str() {
        "5h" => "of 5-hour limit".into(),
        "7d" => "of weekly limit".into(),
        l if l.ends_with(" 7d") => format!("{} weekly", &l[..l.len() - 3]),
        l if l.ends_with(" 5h") => format!("{} 5-hour", &l[..l.len() - 3]),
        l => l.to_string(),
    }
}

fn secondary_label(w: &Window) -> String {
    match w.label.as_str() {
        "7d" => "Weekly".into(),
        "5h" => "5-hour".into(),
        l if l.ends_with(" 7d") => format!("{} weekly", &l[..l.len() - 3]),
        l => l.to_string(),
    }
}

fn bar(p: &Painter, area: Rect, used: f64) {
    p.solid(area, area.h / 2.0, TRACK);
    let fill = area.w * (used as f32 / 100.0);
    if used > 0.0 {
        p.solid(Rect { w: fill.max(area.h), ..area }, area.h / 2.0, severity_color(used));
    }
}

fn plan_name(s: &AgentState) -> Option<String> {
    let p = s.usage.as_ref()?.plan.as_ref()?;
    let mut c = p.chars();
    Some(c.next()?.to_uppercase().chain(c).collect())
}

/// Lays out (and, with a painter, draws) the popover. Returns its height in
/// DIPs and the clickable areas.
pub fn layout(v: &View, p: Option<&Painter>) -> Layout {
    let mut hits = Vec::new();
    let mut y = PAD;

    // Header: title, last update, refresh button, interval chip.
    let interval = format!("{} min", v.config.refresh_minutes);
    let interval_w = text_width(&interval, 11.5) + 18.0;
    let interval_area = r(PAD + INNER - interval_w, y - 1.0, interval_w, 22.0);
    let refresh_area = r(interval_area.x - 30.0, y - 1.0, 26.0, 22.0);
    let latest = v.states.iter().filter_map(|s| s.fetched_at).chain(v.github.fetched_at).max();
    let note = if v.refreshing {
        "Refreshing…".to_string()
    } else if v.paused {
        "Paused (full screen)".to_string()
    } else {
        latest.map(|t| format!("Updated {}", ago(t, v.now))).unwrap_or_default()
    };
    if let Some(p) = p {
        p.text(r(PAD, y, 120.0, 20.0), "Agent usage", Font::Title, TEXT, Align::Left);
        p.text(r(PAD + 110.0, y, refresh_area.x - PAD - 116.0, 20.0), &note, Font::Small, DIM, Align::Right);
        p.round(refresh_area, 11.0, BG, CHIP_OFF_BORDER);
        p.text(refresh_area, ICON_REFRESH, Font::Icon, if v.refreshing { DIM } else { TEXT }, Align::Center);
        p.round(interval_area, 11.0, BG, CHIP_OFF_BORDER);
        p.text(interval_area, &interval, Font::Small, TEXT, Align::Center);
    }
    hits.push((refresh_area, Action::Refresh));
    hits.push((interval_area, Action::CycleInterval));
    y += 30.0;

    let shown: Vec<&AgentState> = v.states.iter().filter(|s| v.config.is_shown(s.id)).collect();
    if shown.is_empty() {
        if let Some(p) = p {
            p.text(r(PAD, y, INNER, 20.0), "Turn on an agent below to see its usage.", Font::Body, DIM, Align::Left);
        }
        y += 28.0;
        divider(p, &mut y);
    }
    for s in shown {
        agent_block(v, p, &mut hits, &mut y, s);
        divider(p, &mut y);
    }

    if v.config.show_github {
        github_block(v, p, &mut hits, &mut y);
        divider(p, &mut y);
    }

    // Chips: which agents (and GitHub) are on.
    let mut x = PAD;
    let chips: Vec<(&str, bool, Action, Option<u32>)> = v
        .states
        .iter()
        .map(|s| (s.id.name(), v.config.is_shown(s.id), Action::ToggleAgent(s.id), Some(agent_color(s.id))))
        .chain(std::iter::once(("GitHub", v.config.show_github, Action::ToggleGithub, None)))
        .collect();
    for (name, on, action, dot) in chips {
        let w = text_width(name, 11.5) + if dot.is_some() { 28.0 } else { 20.0 };
        let area = r(x, y, w, 24.0);
        if let Some(p) = p {
            if on {
                p.round(area, 12.0, CHIP_ON, CHIP_ON_BORDER);
            } else {
                p.round(area, 12.0, BG, CHIP_OFF_BORDER);
            }
            let mut tx = x + 10.0;
            if let Some(c) = dot {
                p.solid(r(x + 9.0, y + 9.0, 6.0, 6.0), 3.0, if on { c } else { DIM });
                tx = x + 19.0;
            }
            p.text(r(tx, y, w - (tx - x), 24.0), name, Font::Small, if on { TEXT } else { DIM }, Align::Left);
        }
        hits.push((area, action));
        x += w + 5.0;
    }
    y += 24.0 + PAD;
    Layout { height: y, hits }
}

fn agent_block(v: &View, p: Option<&Painter>, hits: &mut Vec<(Rect, Action)>, y: &mut f32, s: &AgentState) {
    let main = s.usage.as_ref().and_then(|u| u.windows.first());
    // Name row: dot, name, plan; main window's reset on the right.
    if let Some(p) = p {
        p.solid(r(PAD, *y + 6.0, 8.0, 8.0), 4.0, agent_color(s.id));
        p.text(r(PAD + 14.0, *y, 120.0, 20.0), s.id.name(), Font::Title, TEXT, Align::Left);
        if let Some(plan) = plan_name(s) {
            let x = PAD + 14.0 + text_width(s.id.name(), 13.5) + 8.0;
            p.text(r(x, *y, 80.0, 20.0), &plan, Font::Small, DIM, Align::Left);
        }
        let right = match main {
            Some(w) => w.resets_at.and_then(|t| format_until(t, v.now)).map(|t| format!("resets in {t}")),
            None => match s.status {
                Status::Pending if v.paused => Some("waiting (paused)".into()),
                Status::Pending => Some("loading…".into()),
                Status::SignInRequired => Some("sign-in needed".into()),
                Status::NotFound => Some("not found".into()),
                Status::NotRunning => Some("not running".into()),
                _ => None,
            },
        };
        if let Some(t) = right {
            p.text(r(PAD + 150.0, *y, INNER - 150.0, 20.0), &t, Font::Small, DIM, Align::Right);
        }
    }
    *y += 22.0;

    if let Some(u) = &s.usage
        && let Some(w) = u.windows.first()
    {
        let pct = format!("{:.0}%", w.used_percent);
        if let Some(p) = p {
            let big_w = p.measure(&pct, Font::Big);
            p.text(r(PAD, *y, big_w + 4.0, 34.0), &pct, Font::Big, TEXT, Align::Left);
            p.text(r(PAD + big_w + 10.0, *y + 6.0, INNER - big_w - 10.0, 26.0), &caption(w), Font::Body, DIM, Align::Left);
            bar(p, r(PAD, *y + 38.0, INNER, 6.0), w.used_percent);
        }
        *y += 54.0;
        for w in u.windows.iter().skip(1).take(3) {
            if let Some(p) = p {
                p.text(r(PAD, *y, 180.0, 16.0), &secondary_label(w), Font::Small, DIM, Align::Left);
                let reset = w.resets_at.and_then(|t| format_until(t, v.now));
                let note = match reset {
                    Some(t) => format!("{:.0}%  ·  {t}", w.used_percent),
                    None => format!("{:.0}%", w.used_percent),
                };
                p.text(r(PAD + 150.0, *y, INNER - 150.0, 16.0), &note, Font::Small, TEXT, Align::Right);
                bar(p, r(PAD, *y + 19.0, INNER, 4.0), w.used_percent);
            }
            *y += 31.0;
        }
    }

    if s.status != Status::Ok
        && let Some(msg) = &s.message
    {
        if let Some(p) = p {
            let msg = if s.usage.is_some() { format!("Last data shown: {msg}") } else { msg.clone() };
            p.text(r(PAD, *y, INNER, 18.0), &msg, Font::Small, DIM, Align::Left);
        }
        *y += 20.0;
    }
    if s.id == AgentId::Claude && s.status == Status::SignInRequired {
        let label = if v.signing_in { "Waiting for the browser…" } else { "Sign in with Claude" };
        let area = r(PAD, *y, text_width(label, 12.5) + 8.0, 20.0);
        if let Some(p) = p {
            p.text(area, label, Font::Body, LINK, Align::Left);
        }
        if !v.signing_in {
            hits.push((area, Action::SignInClaude));
        }
        *y += 22.0;
    }
}

fn github_block(v: &View, p: Option<&Painter>, hits: &mut Vec<(Rect, Action)>, y: &mut f32) {
    let gh = v.github;
    let name = gh.data.as_ref().map(|c| format!("GitHub  ·  @{}", c.login)).unwrap_or_else(|| "GitHub".into());
    let right = match &gh.data {
        Some(c) => {
            let today = c.days.last().map(|d| d.count).unwrap_or(0);
            let week: u32 = c.days.iter().rev().take(7).map(|d| d.count).sum();
            format!("Today {today}  ·  7 days {week}")
        }
        None => match gh.status {
            Status::Pending if v.paused => "waiting (paused)".into(),
            Status::Pending => "loading…".into(),
            _ => String::new(),
        },
    };
    if let Some(p) = p {
        p.text(r(PAD, *y, 180.0, 20.0), &name, Font::Title, TEXT, Align::Left);
        p.text(r(PAD + 150.0, *y, INNER - 150.0, 20.0), &right, Font::Small, DIM, Align::Right);
    }
    *y += 26.0;

    if let Some(c) = &gh.data {
        let pitch = INNER / 26.0;
        let cell = pitch - 2.0;
        let weeks = 26;
        // Whole weeks (days start on Sunday), newest on the right.
        let last_week = c.days.len().saturating_sub(1) / 7;
        let first = last_week.saturating_sub(weeks - 1) * 7;
        let days = &c.days[first.min(c.days.len())..];
        let grid = r(PAD, *y, INNER, 7.0 * pitch);
        if let Some(p) = p {
            for (i, d) in days.iter().enumerate() {
                let (col, row) = ((i / 7) as f32, (i % 7) as f32);
                p.solid(r(PAD + col * pitch, *y + row * pitch, cell, cell), 2.0, GH_LEVELS[d.level.min(4) as usize]);
            }
        }
        hits.push((grid, Action::OpenGithub));
        *y += 7.0 * pitch + 6.0;
        if let Some(p) = p {
            let s = format!("{} contributions in the last year", c.total);
            p.text(r(PAD, *y, INNER, 16.0), &s, Font::Small, DIM, Align::Left);
        }
        *y += 18.0;
    } else if let Some(msg) = &gh.message {
        if let Some(p) = p {
            p.text(r(PAD, *y, INNER, 18.0), msg, Font::Small, DIM, Align::Left);
        }
        *y += 20.0;
    }
}

fn divider(p: Option<&Painter>, y: &mut f32) {
    *y += 6.0;
    if let Some(p) = p {
        p.fill(r(PAD, *y, INNER, 1.0), DIVIDER);
    }
    *y += 13.0;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Usage;

    fn view<'a>(states: &'a [AgentState], github: &'a GithubState, config: &'a Config) -> View<'a> {
        View { states, github, config, refreshing: false, paused: false, signing_in: false, now: Timestamp::now() }
    }

    #[test]
    fn chips_and_header_buttons_are_clickable() {
        let states: Vec<AgentState> = AgentId::ALL.iter().map(|a| AgentState::new(*a)).collect();
        let config = Config::default();
        let github = GithubState::default();
        let l = layout(&view(&states, &github, &config), None);
        let chip_y = l.height - PAD - 12.0;
        assert_eq!(l.hit(PAD + 5.0, chip_y), Some(Action::ToggleAgent(AgentId::Claude)));
        assert!(l.hits.iter().any(|(_, a)| *a == Action::ToggleGithub));
        assert!(l.hits.iter().all(|(r, _)| r.x + r.w <= WIDTH - PAD + 0.5), "chips overflow");
        assert_eq!(l.hit(WIDTH - PAD - 5.0, PAD + 5.0), Some(Action::CycleInterval));
    }

    #[test]
    fn blocks_grow_with_windows() {
        let mut states: Vec<AgentState> = AgentId::ALL.iter().map(|a| AgentState::new(*a)).collect();
        let config = Config::default();
        let github = GithubState::default();
        let before = layout(&view(&states, &github, &config), None).height;
        let w = |label: &str| Window { id: label.into(), label: label.into(), used_percent: 50.0, resets_at: None };
        states[0].apply(Ok(Usage { plan: None, source: "test", windows: vec![w("5h"), w("7d")] }), Timestamp::now());
        assert!(layout(&view(&states, &github, &config), None).height > before + 50.0);
        assert_eq!(caption(&w("5h")), "of 5-hour limit");
        assert_eq!(caption(&w("Gemini 7d")), "Gemini weekly");
        assert_eq!(secondary_label(&w("7d")), "Weekly");
    }
}
