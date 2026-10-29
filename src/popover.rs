//! The popover's content: one row per agent with its on/off toggle and bars,
//! then the shared GitHub section with a contribution graph, then buttons.
//! `layout` is used both to size/hit-test (no device context) and to paint.

use crate::config::Config;
use crate::github::GithubState;
use crate::icon::severity_color;
use crate::model::{AgentId, AgentState, Status, format_until};
use jiff::Timestamp;
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, CreatePen, CreateSolidBrush, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawTextW, FillRect, HDC, HFONT, HGDIOBJ, PS_SOLID, RoundRect, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT, FW_NORMAL, FW_SEMIBOLD, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS,
    CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, DEFAULT_PITCH, FF_DONTCARE,
};
use windows::core::w;

pub const WIDTH: f32 = 340.0;
const PAD: f32 = 14.0;

const BG: u32 = 0x1F1F1F;
const TEXT: u32 = 0xF2F2F2;
const DIM: u32 = 0x9A9A9A;
const TRACK: u32 = 0x3A3A3A;
const DIVIDER: u32 = 0x333333;
const ON: u32 = 0x0A84FF;
const OFF: u32 = 0x555555;
const LINK: u32 = 0x4DA3FF;
/// GitHub's dark-theme contribution shades, level 0..=4.
const GH_LEVELS: [u32; 5] = [0x2D333B, 0x0E4429, 0x006D32, 0x26A641, 0x39D353];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleAgent(AgentId),
    ToggleGithub,
    SignInClaude,
    OpenGithub,
    Refresh,
    CycleInterval,
    Quit,
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

pub struct Layout {
    pub height: f32,
    pub hits: Vec<(Rect, Action)>,
}

impl Layout {
    pub fn hit(&self, x: f32, y: f32) -> Option<Action> {
        self.hits.iter().find(|(r, _)| r.contains(x, y)).map(|(_, a)| *a)
    }
}

pub struct Painter {
    hdc: HDC,
    scale: f32,
    fonts: [HFONT; 2],
}

#[derive(Clone, Copy)]
enum Font {
    Bold,
    Small,
}

#[derive(Clone, Copy)]
enum Align {
    Left,
    Center,
    Right,
}

fn colorref(rgb: u32) -> COLORREF {
    COLORREF(((rgb & 0xFF) << 16) | (rgb & 0xFF00) | ((rgb >> 16) & 0xFF))
}

impl Painter {
    pub fn new(hdc: HDC, scale: f32) -> Painter {
        let font = |size: f32, weight: u32| unsafe {
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
                CLEARTYPE_QUALITY,
                (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
                w!("Segoe UI"),
            )
        };
        unsafe { SetBkMode(hdc, TRANSPARENT) };
        Painter {
            hdc,
            scale,
            fonts: [font(13.0, FW_SEMIBOLD.0), font(11.5, FW_NORMAL.0)],
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

    fn round(&self, r: Rect, radius: f32, rgb: u32) {
        let rc = self.rect(r);
        unsafe {
            let b = CreateSolidBrush(colorref(rgb));
            let p = CreatePen(PS_SOLID, 1, colorref(rgb));
            let ob = SelectObject(self.hdc, HGDIOBJ(b.0));
            let op = SelectObject(self.hdc, HGDIOBJ(p.0));
            let d = self.px(radius * 2.0).max(1);
            let _ = RoundRect(self.hdc, rc.left, rc.top, rc.right, rc.bottom, d, d);
            SelectObject(self.hdc, ob);
            SelectObject(self.hdc, op);
            let _ = DeleteObject(b.into());
            let _ = DeleteObject(p.into());
        }
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

fn r(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect { x, y, w, h }
}

fn status_text(s: &AgentState, shown: bool, paused: bool) -> String {
    if !shown {
        return "Hidden".into();
    }
    match s.status {
        Status::Pending if paused => "Waiting (paused)".into(),
        Status::Pending => "Loading…".into(),
        Status::Ok => s.usage.as_ref().and_then(|u| u.plan.clone()).map(capitalize).unwrap_or_default(),
        Status::SignInRequired => "Sign-in needed".into(),
        Status::NotFound => "Not found".into(),
        Status::NotRunning => "Not running".into(),
        Status::Error => "Error".into(),
    }
}

fn capitalize(s: String) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => s,
    }
}

fn ago(t: Timestamp, now: Timestamp) -> String {
    let mins = (now.as_second() - t.as_second()) / 60;
    match mins {
        i64::MIN..=0 => "just now".into(),
        1..=59 => format!("{mins}m ago"),
        _ => format!("{}h ago", mins / 60),
    }
}

/// Lays out (and, with a painter, draws) the popover. Returns its height in
/// DIPs and the clickable areas.
pub fn layout(v: &View, p: Option<&Painter>) -> Layout {
    let mut hits = Vec::new();
    let inner = WIDTH - 2.0 * PAD;
    let mut y = PAD;

    // Header.
    let latest = v.states.iter().filter_map(|s| s.fetched_at).chain(v.github.fetched_at).max();
    let header_note = if v.refreshing {
        "Refreshing…".to_string()
    } else if v.paused {
        "Paused (full-screen app)".to_string()
    } else {
        latest.map(|t| format!("Updated {}", ago(t, v.now))).unwrap_or_default()
    };
    if let Some(p) = p {
        p.text(r(PAD, y, inner, 20.0), "Agent usage", Font::Bold, TEXT, Align::Left);
        p.text(r(PAD, y, inner, 20.0), &header_note, Font::Small, DIM, Align::Right);
    }
    y += 28.0;

    for s in v.states {
        let shown = v.config.is_shown(s.id);
        toggle_row(p, &mut hits, y, shown, s.id.name(), &status_text(s, shown, v.paused), Action::ToggleAgent(s.id));
        y += 28.0;
        if !shown {
            continue;
        }
        if let Some(u) = &s.usage {
            for w in u.windows.iter().take(4) {
                if let Some(p) = p {
                    let bar = r(PAD + 96.0, y + 7.0, 104.0, 6.0);
                    p.text(r(PAD + 8.0, y, 86.0, 20.0), &w.label, Font::Small, DIM, Align::Left);
                    p.round(bar, 3.0, TRACK);
                    let fill = (bar.w * (w.used_percent as f32 / 100.0)).max(if w.used_percent > 0.0 { 3.0 } else { 0.0 });
                    if fill > 0.0 {
                        p.round(Rect { w: fill, ..bar }, 3.0, severity_color(w.used_percent));
                    }
                    let reset = w.resets_at.and_then(|t| format_until(t, v.now));
                    let note = match reset {
                        Some(r) => format!("{:.0}%  ·  {r}", w.used_percent),
                        None => format!("{:.0}%", w.used_percent),
                    };
                    p.text(r(PAD + 204.0, y, inner - 204.0, 20.0), &note, Font::Small, TEXT, Align::Right);
                }
                y += 20.0;
            }
        }
        if s.status != Status::Ok
            && let Some(msg) = &s.message
        {
            let stale = s.usage.is_some();
            if let Some(p) = p {
                let msg = if stale { format!("Showing last data: {msg}") } else { msg.clone() };
                p.text(r(PAD + 8.0, y, inner - 8.0, 18.0), &msg, Font::Small, DIM, Align::Left);
            }
            y += 18.0;
        }
        if s.id == AgentId::Claude && s.status == Status::SignInRequired {
            let label = if v.signing_in { "Waiting for the browser…" } else { "Sign in with Claude…" };
            let area = r(PAD + 8.0, y, 180.0, 20.0);
            if let Some(p) = p {
                p.text(area, label, Font::Small, LINK, Align::Left);
            }
            if !v.signing_in {
                hits.push((area, Action::SignInClaude));
            }
            y += 22.0;
        }
        y += 4.0;
    }

    // Shared GitHub section.
    divider(p, &mut y, inner);
    let gh = v.github;
    let gh_note = if !v.config.show_github {
        "Hidden".to_string()
    } else if let Some(c) = &gh.data {
        format!("@{}  ·  {} this year", c.login, c.total)
    } else {
        match gh.status {
            Status::Pending if v.paused => "Waiting (paused)".into(),
            Status::Pending => "Loading…".into(),
            _ => "Unavailable".into(),
        }
    };
    toggle_row(p, &mut hits, y, v.config.show_github, "GitHub", &gh_note, Action::ToggleGithub);
    y += 30.0;
    if v.config.show_github {
        if let Some(c) = &gh.data {
            let cell = 9.0;
            let pitch = 11.0;
            let weeks = ((inner + (pitch - cell)) / pitch).floor() as usize;
            // Whole weeks (days start on Sunday), newest on the right.
            let last_week = c.days.len().saturating_sub(1) / 7;
            let first = last_week.saturating_sub(weeks - 1) * 7;
            let days = &c.days[first.min(c.days.len())..];
            let grid = r(PAD, y, inner, 7.0 * pitch);
            if let Some(p) = p {
                for (i, d) in days.iter().enumerate() {
                    let (col, row) = (i / 7, i % 7);
                    let cx = PAD + col as f32 * pitch;
                    p.round(r(cx, y + row as f32 * pitch, cell, cell), 2.0, GH_LEVELS[d.level.min(4) as usize]);
                }
            }
            hits.push((grid, Action::OpenGithub));
            y += 7.0 * pitch + 4.0;
            let week: u32 = days.iter().rev().take(7).map(|d| d.count).sum();
            let today = c.days.last().map(|d| d.count).unwrap_or(0);
            if let Some(p) = p {
                let s = format!("Today {today}  ·  last 7 days {week}");
                p.text(r(PAD, y, inner, 18.0), &s, Font::Small, DIM, Align::Left);
                // Legend: Less [5 shades] More, right-aligned like github.com.
                let right = PAD + inner;
                p.text(r(right - 28.0, y, 28.0, 18.0), "More", Font::Small, DIM, Align::Right);
                for (i, shade) in GH_LEVELS.iter().enumerate() {
                    let x = right - 32.0 - (5 - i) as f32 * pitch;
                    p.round(r(x, y + 4.5, cell, cell), 2.0, *shade);
                }
                p.text(r(right - 32.0 - 5.0 * pitch - 34.0, y, 30.0, 18.0), "Less", Font::Small, DIM, Align::Right);
            }
            y += 20.0;
        } else if let Some(msg) = &gh.message {
            if let Some(p) = p {
                p.text(r(PAD + 8.0, y, inner - 8.0, 18.0), msg, Font::Small, DIM, Align::Left);
            }
            y += 20.0;
        }
    }

    // Buttons.
    divider(p, &mut y, inner);
    let buttons = [
        ("Refresh", Action::Refresh),
        (&*format!("Every {} min", v.config.refresh_minutes), Action::CycleInterval),
        ("Quit", Action::Quit),
    ];
    let bw = inner / buttons.len() as f32;
    for (i, (label, action)) in buttons.iter().enumerate() {
        let area = r(PAD + i as f32 * bw, y, bw, 26.0);
        if let Some(p) = p {
            p.round(r(area.x + 3.0, area.y, area.w - 6.0, area.h), 6.0, 0x2C2C2C);
            p.text(area, label, Font::Small, TEXT, Align::Center);
        }
        hits.push((area, *action));
    }
    y += 26.0 + PAD;
    Layout { height: y, hits }
}

fn divider(p: Option<&Painter>, y: &mut f32, inner: f32) {
    *y += 4.0;
    if let Some(p) = p {
        p.fill(r(PAD, *y, inner, 1.0), DIVIDER);
    }
    *y += 9.0;
}

fn toggle_row(p: Option<&Painter>, hits: &mut Vec<(Rect, Action)>, y: f32, on: bool, name: &str, note: &str, action: Action) {
    let inner = WIDTH - 2.0 * PAD;
    if let Some(p) = p {
        let pill = r(PAD, y + 3.0, 32.0, 18.0);
        p.round(pill, 9.0, if on { ON } else { OFF });
        let knob_x = if on { pill.x + pill.w - 16.0 } else { pill.x + 2.0 };
        p.round(r(knob_x, pill.y + 2.0, 14.0, 14.0), 7.0, 0xFFFFFF);
        p.text(r(PAD + 42.0, y, 140.0, 24.0), name, Font::Bold, if on { TEXT } else { DIM }, Align::Left);
        p.text(r(PAD + 150.0, y, inner - 150.0, 24.0), note, Font::Small, DIM, Align::Right);
    }
    // The whole row toggles, not only the pill.
    hits.push((r(PAD, y, 150.0, 24.0), action));
}

pub fn background() -> u32 {
    BG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_testing_finds_toggles_and_buttons() {
        let states: Vec<AgentState> = AgentId::ALL.iter().map(|a| AgentState::new(*a)).collect();
        let config = Config::default();
        let github = GithubState::default();
        let v = View { states: &states, github: &github, config: &config, refreshing: false, paused: false, signing_in: false, now: Timestamp::now() };
        let l = layout(&v, None);
        assert!(l.height > 150.0);
        assert_eq!(l.hit(PAD + 5.0, PAD + 28.0 + 10.0), Some(Action::ToggleAgent(AgentId::Claude)));
        assert_eq!(l.hit(PAD + 5.0, l.height - PAD - 10.0), Some(Action::Refresh));
        assert_eq!(l.hit(WIDTH - PAD - 5.0, l.height - PAD - 10.0), Some(Action::Quit));
        assert!(l.hits.iter().any(|(_, a)| *a == Action::ToggleGithub));
    }
}
