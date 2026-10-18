// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dv-subsystem-set contributors
//! The setup window the dv apps' installers share: one page at a time (choose, installing, done), drawn by itself in
//! the system's light or dark look, with the keys a dialog has (Tab, Space, Enter, Esc). What is installed comes from
//! the app as a `Product` and a `Work`, so the window knows nothing of any one app.
#[cfg(feature = "net")]
pub mod net;

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush, DeleteDC,
    DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect, GetMonitorInfoW, InvalidateRect, MonitorFromWindow,
    SelectObject, SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DRAW_TEXT_FORMAT,
    DT_CENTER, DT_END_ELLIPSIS, DT_NOPREFIX, DT_PATH_ELLIPSIS, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, HDC, HFONT,
    MONITORINFO, MONITOR_DEFAULTTOPRIMARY, OUT_DEFAULT_PRECIS, PAINTSTRUCT, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_DOWN, VK_ESCAPE,
    VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyWindow, DispatchMessageW, DrawIconEx, GetClientRect,
    GetMessageW, LoadCursorW, LoadIconW, LoadImageW, PostMessageW, PostQuitMessage, RegisterClassExW, SetWindowPos,
    ShowWindow, TranslateMessage, DI_NORMAL, HICON, IDC_ARROW, IMAGE_ICON, LR_DEFAULTCOLOR, MSG, SWP_NOACTIVATE,
    SWP_NOZORDER, SW_SHOW, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETTINGCHANGE, WNDCLASSEXW,
    WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};

/// one box on the first page
pub struct Choice {
    pub label: String,
    pub on: bool,
    /// the box this one needs: while this is ticked, that one is ticked too and can't be cleared
    pub needs: Option<usize>,
}

/// what the window installs: one app in a version, or several apps (each box one of them)
pub enum Scope {
    App {
        version: &'static str,
        /// the version installed now, if any
        installed: Option<String>,
    },
    /// the line above the boxes, "dvfiles has no release yet."
    Apps { note: Option<String> },
}

/// what is being installed, as the window shows it
pub struct Product {
    /// "dvfiles", or for several apps "the dv apps"
    pub name: &'static str,
    pub scope: Scope,
    /// the line under the title, "A fast file manager for Windows"
    pub about: &'static str,
    pub location: PathBuf,
    pub choices: Vec<Choice>,
    /// what the last page says, "Find it in the Start menu..."
    pub done: String,
    /// the last page's box that starts the app, "Open dvfiles now"; none for an app the install starts itself
    pub launch: Option<String>,
    /// false shows the window without taking the keyboard (for tests)
    pub activate: bool,
}

/// the installing itself, run on a thread of its own
pub trait Work: Send + Sync + 'static {
    /// installs with the boxes as `ticked`, telling `step` what it is doing and how far along it is (0 to 1); gives
    /// the installed exe
    fn install(&self, ticked: &[bool], step: &dyn Fn(&str, f32)) -> Result<PathBuf, String>;
    fn launch(&self, exe: &Path);
}

/// where `a` stands to `b` in version order: "0.2.10" is after "0.2.9"
pub fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    parts(a) > parts(b)
}

// the client area in 96-dpi pixels; the bands at the top and bottom
const W: i32 = 560;
const H: i32 = 430;
const HEAD: i32 = 92;
const FOOT: i32 = 60;
const ROW: i32 = 32;
const WM_STEP: u32 = WM_APP + 1;
const WM_FINISHED: u32 = WM_APP + 2;

/// the installed version against the one being installed
#[derive(Clone, Copy, PartialEq, Debug)]
enum Against {
    Same,
    /// the installed one is newer: this replaces it with an older one
    Older,
    Newer,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Rgb(u8, u8, u8);

impl Rgb {
    const fn hex(v: u32) -> Rgb {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    fn mix(self, o: Rgb, t: f32) -> Rgb {
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Rgb(m(self.0, o.0), m(self.1, o.1), m(self.2, o.2))
    }

    fn cr(self) -> COLORREF {
        COLORREF(self.0 as u32 | (self.1 as u32) << 8 | (self.2 as u32) << 16)
    }

    fn luma(self) -> u32 {
        (self.0 as u32 * 77 + self.1 as u32 * 150 + self.2 as u32 * 29) >> 8
    }
}

/// the colours, as dvfiles has them
struct Pal {
    dark: bool,
    chrome: Rgb,
    bg: Rgb,
    line: Rgb,
    fg: Rgb,
    muted: Rgb,
    danger: Rgb,
    accent: Rgb,
    accent_hot: Rgb,
    accent_press: Rgb,
    on_accent: Rgb,
    mark: Rgb,
    btn: Rgb,
    btn_hot: Rgb,
    btn_press: Rgb,
    btn_border: Rgb,
}

impl Pal {
    fn system() -> Pal {
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
        let light: u32 = hkcu
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
            .and_then(|k| k.get_value("AppsUseLightTheme"))
            .unwrap_or(1);
        // stored as 0xAABBGGRR
        let accent = hkcu
            .open_subkey(r"Software\Microsoft\Windows\DWM")
            .and_then(|k| k.get_value::<u32, _>("AccentColor"))
            .map_or(Rgb::hex(0x0078D7), |c| Rgb(c as u8, (c >> 8) as u8, (c >> 16) as u8));
        let white = Rgb::hex(0xFFFFFF);
        if light == 0 {
            // lifted towards white until it stands out on the dark surfaces, as dvfiles does
            let l = accent.luma();
            let accent = accent.mix(
                white,
                if l < 90 {
                    0.5
                } else if l < 150 {
                    0.35
                } else {
                    0.1
                },
            );
            let chrome = Rgb::hex(0x222222);
            Pal {
                dark: true,
                chrome,
                bg: Rgb::hex(0x191919),
                line: Rgb::hex(0x303030),
                fg: Rgb::hex(0xEDEDED),
                muted: Rgb::hex(0xA3A3A3),
                danger: Rgb::hex(0xE8705F),
                accent,
                accent_hot: accent.mix(white, 0.12),
                accent_press: accent.mix(Rgb(0, 0, 0), 0.12),
                on_accent: if accent.luma() > 140 { Rgb::hex(0x111111) } else { white },
                mark: Rgb::hex(0x9A9A9A),
                btn: Rgb::hex(0x2D2D2D),
                btn_hot: Rgb::hex(0x383838),
                btn_press: Rgb::hex(0x262626),
                btn_border: Rgb::hex(0x404040),
            }
        } else {
            let accent = accent.mix(Rgb(0, 0, 0), if accent.luma() > 170 { 0.35 } else { 0.0 });
            Pal {
                dark: false,
                chrome: Rgb::hex(0xEDEEF0),
                bg: white,
                line: Rgb::hex(0xDDDEE1),
                fg: Rgb::hex(0x1B1B1B),
                muted: Rgb::hex(0x5F6368),
                danger: Rgb::hex(0xC42B1C),
                accent,
                accent_hot: accent.mix(Rgb(0, 0, 0), 0.12),
                accent_press: accent.mix(Rgb(0, 0, 0), 0.22),
                on_accent: white,
                mark: Rgb::hex(0x6B7078),
                btn: Rgb::hex(0xFBFBFC),
                btn_hot: Rgb::hex(0xF2F3F5),
                btn_press: Rgb::hex(0xE6E7EA),
                btn_border: Rgb::hex(0xD0D2D6),
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Choice(usize),
    Launch,
    Primary,
    Secondary,
}

enum Page {
    Choose,
    Working { step: String, done: f32 },
    Done(PathBuf),
    Failed(String),
}

struct Fonts {
    body: HFONT,
    title: HFONT,
    // Segoe MDL2 Assets, for the check mark: drawn as a glyph it gets ClearType's smooth edges
    glyph: HFONT,
}

struct Ui {
    hwnd: HWND,
    p: Product,
    work: Arc<dyn Work>,
    page: Page,
    ticked: Vec<bool>,
    launch: bool,
    hot: Option<Hit>,
    press: Option<Hit>,
    focus: Hit,
    // focus marks show once the keyboard is used, as in Windows' own dialogs
    keys: bool,
    dpi: u32,
    pal: Pal,
    fonts: Fonts,
    icon: HICON,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn font(px: i32, weight: i32, face: &str) -> HFONT {
    let face = wide(face);
    unsafe {
        CreateFontW(
            -px,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            PCWSTR(face.as_ptr()),
        )
    }
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> RECT {
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

fn fill(dc: HDC, r: RECT, c: Rgb) {
    unsafe {
        let b = CreateSolidBrush(c.cr());
        FillRect(dc, &r, b);
        let _ = DeleteObject(b.into());
    }
}

fn frame(dc: HDC, r: RECT, c: Rgb) {
    unsafe {
        let b = CreateSolidBrush(c.cr());
        FrameRect(dc, &r, b);
        let _ = DeleteObject(b.into());
    }
}

fn text(dc: HDC, s: &str, mut r: RECT, f: HFONT, c: Rgb, flags: DRAW_TEXT_FORMAT) {
    // an empty slice's pointer is dangling, and DrawTextW reads it even for no characters
    if s.is_empty() {
        return;
    }
    let mut t: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        SelectObject(dc, f.into());
        SetTextColor(dc, c.cr());
        DrawTextW(dc, &mut t, &mut r, flags | DT_NOPREFIX);
    }
}

/// a boxed value through the message queue; dropped here when the window is gone
fn post_box<T>(hwnd: HWND, msg: u32, w: usize, v: T) {
    let p = Box::into_raw(Box::new(v));
    unsafe {
        if PostMessageW(Some(hwnd), msg, WPARAM(w), LPARAM(p as isize)).is_err() {
            drop(Box::from_raw(p));
        }
    }
}

impl Ui {
    fn px(&self, v: i32) -> i32 {
        (v * self.dpi as i32 + 48) / 96
    }

    fn load_look(&mut self) {
        let s = |v| self.px(v);
        let fonts = Fonts {
            body: font(s(14), 400, "Segoe UI"),
            title: font(s(22), 600, "Segoe UI"),
            glyph: font(s(12), 400, "Segoe MDL2 Assets"),
        };
        let size = s(48);
        let icon = unsafe {
            let inst = GetModuleHandleW(None).unwrap_or_default();
            LoadImageW(Some(inst.into()), PCWSTR(1 as *const u16), IMAGE_ICON, size, size, LR_DEFAULTCOLOR)
                .map(|h| HICON(h.0))
                .unwrap_or_default()
        };
        self.free_look();
        self.fonts = fonts;
        self.icon = icon;
    }

    fn free_look(&mut self) {
        unsafe {
            for f in [self.fonts.body, self.fonts.title, self.fonts.glyph] {
                if !f.is_invalid() {
                    let _ = DeleteObject(f.into());
                }
            }
            if !self.icon.is_invalid() {
                let _ = DestroyIcon(self.icon);
            }
        }
    }

    /// the caption in the header's colour, so the two read as one band (Windows 11; Windows 10 keeps its own)
    fn tint_caption(&self) {
        let dark = self.pal.dark as i32;
        let cap = self.pal.chrome.cr();
        unsafe {
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const i32 as *const _, 4);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_CAPTION_COLOR, &cap as *const COLORREF as *const _, 4);
        }
    }

    /// a box is locked on while a ticked box needs it
    fn locked(&self, i: usize) -> bool {
        self.p.choices.iter().zip(&self.ticked).any(|(c, &t)| t && c.needs == Some(i))
    }

    fn effective(&self) -> Vec<bool> {
        (0..self.ticked.len()).map(|i| self.ticked[i] || self.locked(i)).collect()
    }

    fn note(&self) -> Option<(String, bool)> {
        let (version, v) = match &self.p.scope {
            Scope::App { version, installed } => (*version, installed.as_deref()?),
            Scope::Apps { note } => return note.clone().map(|n| (n, false)),
        };
        Some(if v == version {
            ("This version is installed already; installing it again repairs it.".into(), false)
        } else if is_newer(v, version) {
            (format!("A newer version, {v}, is installed. Your settings are kept."), true)
        } else {
            (format!("Version {v} is installed. Your settings are kept."), false)
        })
    }

    /// how this install stands to what is there: None for a fresh install (and for several apps)
    fn against(&self) -> Option<Against> {
        let Scope::App { version, installed: Some(i) } = &self.p.scope else { return None };
        Some(if i == version {
            Against::Same
        } else if is_newer(i, version) {
            Against::Older
        } else {
            Against::Newer
        })
    }

    fn rows_top(&self) -> i32 {
        if self.note().is_some() {
            196
        } else {
            172
        }
    }

    fn title(&self) -> String {
        let n = self.p.name;
        let (v, i) = match &self.p.scope {
            Scope::App { version, installed } => (format!(" {version}"), installed.as_deref().unwrap_or_default()),
            Scope::Apps { .. } => (String::new(), ""),
        };
        match (&self.page, self.against()) {
            (Page::Choose, None) => format!("Install {n}{v}"),
            (Page::Choose, Some(Against::Same)) => format!("Reinstall {n}{v}"),
            (Page::Choose, Some(Against::Older)) => format!("Replace {n} {i} with{v}"),
            (Page::Choose, Some(Against::Newer)) => format!("Update {n} to{v}"),
            (Page::Working { .. }, _) => format!("Installing {n}..."),
            (Page::Done(_), None) if v.is_empty() => format!("{n} are installed"),
            (Page::Done(_), None) => format!("{n} is installed"),
            (Page::Done(_), Some(_)) => format!("{n} is now version{v}"),
            (Page::Failed(_), _) => "Setup could not finish".into(),
        }
    }

    fn primary_label(&self) -> &'static str {
        match (&self.page, self.against()) {
            (Page::Choose, None) => "Install",
            (Page::Choose, Some(Against::Same)) => "Reinstall",
            (Page::Choose, Some(Against::Older)) => "Replace",
            (Page::Choose, Some(Against::Newer)) => "Update",
            (Page::Done(_), _) => "Finish",
            _ => "Close",
        }
    }

    /// what can be clicked or focused on this page, in Tab order
    fn items(&self) -> Vec<(Hit, RECT)> {
        let s = |v| self.px(v);
        let (w, h) = (s(W), s(H));
        let row = |y: i32| RECT { left: s(20), top: s(y), right: w - s(20), bottom: s(y + ROW) };
        let mut v = Vec::new();
        match self.page {
            Page::Choose => {
                let top = self.rows_top();
                v.extend((0..self.p.choices.len()).map(|i| (Hit::Choice(i), row(top + ROW * i as i32))));
            }
            Page::Done(_) if self.p.launch.is_some() => v.push((Hit::Launch, row(200))),
            _ => {}
        }
        let (bw, bh) = (s(104), s(30));
        let by = h - s(FOOT) + (s(FOOT) - bh) / 2;
        let right = w - s(20);
        match self.page {
            Page::Choose => {
                v.push((Hit::Primary, rect(right - 2 * bw - s(8), by, bw, bh)));
                v.push((Hit::Secondary, rect(right - bw, by, bw, bh)));
            }
            Page::Done(_) | Page::Failed(_) => v.push((Hit::Primary, rect(right - bw, by, bw, bh))),
            Page::Working { .. } => {}
        }
        v
    }

    fn hit(&self, x: i32, y: i32) -> Option<Hit> {
        self.items()
            .into_iter()
            .find(|(_, r)| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
            .map(|(h, _)| h)
            .filter(|&h| !matches!(h, Hit::Choice(i) if self.locked(i)))
    }

    fn paint(&self, dc: HDC) {
        let s = |v| self.px(v);
        let (w, h) = (s(W), s(H));
        let pal = &self.pal;
        fill(dc, rect(0, 0, w, s(HEAD)), pal.chrome);
        fill(dc, rect(0, s(HEAD), w, h - s(HEAD) - s(FOOT)), pal.bg);
        fill(dc, rect(0, h - s(FOOT), w, s(FOOT)), pal.chrome);
        fill(dc, rect(0, h - s(FOOT), w, 1), pal.line);
        unsafe {
            SetBkMode(dc, TRANSPARENT);
            let _ = DrawIconEx(dc, s(26), s(22), self.icon, s(48), s(48), 0, None, DI_NORMAL);
        }
        let one = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS;
        let f = &self.fonts;
        text(dc, &self.title(), rect(s(92), s(18), w - s(112), s(32)), f.title, pal.fg, one);
        text(dc, self.p.about, rect(s(92), s(50), w - s(112), s(22)), f.body, pal.muted, one);
        let body = |y: i32, hgt: i32| rect(s(28), s(y), w - s(56), s(hgt));
        match &self.page {
            Page::Choose => {
                text(dc, "Folder", body(110, 22), f.body, pal.muted, one);
                let loc = self.p.location.display().to_string();
                let r = RECT { left: s(92), ..body(110, 22) };
                text(dc, &loc, r, f.body, pal.fg, DT_SINGLELINE | DT_VCENTER | DT_PATH_ELLIPSIS);
                let line = "For your account only, no administrator rights needed.";
                text(dc, line, body(134, 22), f.body, pal.muted, one);
                if let Some((note, warn)) = self.note() {
                    text(dc, &note, body(158, 22), f.body, if warn { pal.danger } else { pal.fg }, one);
                }
            }
            Page::Working { step, done } => {
                text(dc, step, body(110, 22), f.body, pal.muted, one);
                let bar = body(146, 4);
                fill(dc, bar, pal.line);
                let filled = ((bar.right - bar.left) as f32 * done.clamp(0.0, 1.0)) as i32;
                fill(dc, RECT { right: bar.left + filled, ..bar }, pal.accent);
            }
            Page::Done(_) => text(dc, &self.p.done, body(108, 84), f.body, pal.fg, DT_WORDBREAK),
            Page::Failed(e) => text(dc, e, body(108, H - 108 - FOOT - 16), f.body, pal.fg, DT_WORDBREAK),
        }
        let ticked = self.effective();
        for (hit, r) in self.items() {
            let (hot, pressed) = (self.hot == Some(hit), self.press == Some(hit) && self.hot == Some(hit));
            match hit {
                Hit::Choice(_) | Hit::Launch => {
                    let (label, on, locked) = match hit {
                        Hit::Choice(i) => (self.p.choices[i].label.as_str(), ticked[i], self.locked(i)),
                        _ => (self.p.launch.as_deref().unwrap_or_default(), self.launch, false),
                    };
                    let b = s(16);
                    let bx = rect(r.left + s(8), r.top + (r.bottom - r.top - b) / 2, b, b);
                    if on {
                        let c = if locked {
                            pal.mark
                        } else if pressed {
                            pal.accent_press
                        } else if hot {
                            pal.accent_hot
                        } else {
                            pal.accent
                        };
                        fill(dc, bx, c);
                        let mark = if locked { pal.bg } else { pal.on_accent };
                        text(dc, "\u{E73E}", bx, f.glyph, mark, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                    } else {
                        fill(dc, bx, if pressed { pal.btn_press } else { pal.bg });
                        frame(dc, bx, if hot { pal.fg } else { pal.mark });
                    }
                    let lr = RECT { left: bx.right + s(12), ..r };
                    text(dc, label, lr, f.body, if locked { pal.muted } else { pal.fg }, one);
                }
                Hit::Primary | Hit::Secondary => {
                    let primary = hit == Hit::Primary;
                    let (bg, fg) = match (primary, pressed, hot) {
                        (true, true, _) => (pal.accent_press, pal.on_accent),
                        (true, _, true) => (pal.accent_hot, pal.on_accent),
                        (true, _, _) => (pal.accent, pal.on_accent),
                        (false, true, _) => (pal.btn_press, pal.fg),
                        (false, _, true) => (pal.btn_hot, pal.fg),
                        (false, _, _) => (pal.btn, pal.fg),
                    };
                    fill(dc, r, bg);
                    if !primary {
                        frame(dc, r, pal.btn_border);
                    }
                    let label = if primary { self.primary_label() } else { "Cancel" };
                    text(dc, label, r, f.body, fg, DT_SINGLELINE | DT_VCENTER | DT_CENTER);
                }
            }
            if self.keys && self.focus == hit {
                let inset = if matches!(hit, Hit::Primary | Hit::Secondary) { s(3) } else { 0 };
                let fr =
                    RECT { left: r.left + inset, top: r.top + inset, right: r.right - inset, bottom: r.bottom - inset };
                let c = if hit == Hit::Primary { pal.on_accent } else { pal.fg };
                frame(dc, fr, c);
            }
        }
    }

    fn start(&mut self) {
        self.page = Page::Working { step: String::new(), done: 0.0 };
        self.hot = None;
        let ticked = self.effective();
        let work = self.work.clone();
        // a window handle is not Send; the number is
        let hwnd = self.hwnd.0 as isize;
        std::thread::spawn(move || {
            let hwnd = HWND(hwnd as *mut _);
            let step = |t: &str, f: f32| post_box(hwnd, WM_STEP, (f * 1000.0) as usize, t.to_string());
            let r = work.install(&ticked, &step);
            post_box(hwnd, WM_FINISHED, 0, r);
        });
    }

    /// what a click or Space does; true when the window is to close
    fn activate(&mut self, hit: Hit) -> bool {
        match hit {
            Hit::Choice(i) if !self.locked(i) => self.ticked[i] = !self.ticked[i],
            Hit::Choice(_) => {}
            Hit::Launch => self.launch = !self.launch,
            Hit::Primary => match &self.page {
                Page::Choose => self.start(),
                Page::Done(exe) => {
                    if self.launch && self.p.launch.is_some() {
                        self.work.launch(exe);
                    }
                    return true;
                }
                Page::Failed(_) => return true,
                Page::Working { .. } => {}
            },
            Hit::Secondary => return true,
        }
        false
    }

    fn key(&mut self, vk: u16) -> bool {
        let items: Vec<Hit> = self.items().into_iter().map(|(h, _)| h).filter(|&h| self.hit_ok(h)).collect();
        if items.is_empty() {
            return false;
        }
        let at = items.iter().position(|&h| h == self.focus).unwrap_or(0);
        let back = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        match vk {
            v if v == VK_TAB.0 || v == VK_DOWN.0 || v == VK_UP.0 => {
                let prev = if v == VK_TAB.0 { back } else { v == VK_UP.0 };
                let n = items.len();
                self.focus = items[if prev { (at + n - 1) % n } else { (at + 1) % n }];
                self.keys = true;
            }
            v if v == VK_SPACE.0 => {
                self.keys = true;
                return self.activate(items[at]);
            }
            v if v == VK_RETURN.0 => {
                let hit = if self.focus == Hit::Secondary { Hit::Secondary } else { Hit::Primary };
                return self.activate(hit);
            }
            v if v == VK_ESCAPE.0 => return !matches!(self.page, Page::Working { .. }),
            _ => {}
        }
        false
    }

    fn hit_ok(&self, h: Hit) -> bool {
        !matches!(h, Hit::Choice(i) if self.locked(i))
    }
}

impl Drop for Ui {
    fn drop(&mut self) {
        self.free_look();
    }
}

/// sized for its dpi and centred on its screen's work area
fn place(hwnd: HWND, dpi: u32, style: u32) {
    let px = |v: i32| (v * dpi as i32 + 48) / 96;
    let mut r = rect(0, 0, px(W), px(H));
    unsafe {
        let _ = AdjustWindowRectExForDpi(
            &mut r,
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(style),
            false,
            WINDOW_EX_STYLE(0),
            dpi,
        );
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY), &mut mi);
        let wa = mi.rcWork;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let (x, y) = (wa.left + (wa.right - wa.left - w) / 2, wa.top + (wa.bottom - wa.top - h) / 2);
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Shows the setup window until it is closed.
pub fn run(p: Product, work: impl Work) {
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let activate = p.activate;
    let title = wide(&format!("{} Setup", p.name));
    unsafe {
        let inst: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        let class = w!("dvSetup");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc),
            hInstance: inst,
            hIcon: LoadIconW(Some(inst), PCWSTR(1 as *const u16)).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: class,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let Ok(hwnd) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(title.as_ptr()),
            style,
            0,
            0,
            100,
            100,
            None,
            None,
            Some(inst),
            None,
        ) else {
            return;
        };
        let dpi = GetDpiForWindow(hwnd).max(96);
        let ticked = p.choices.iter().map(|c| c.on).collect();
        let mut ui = Ui {
            hwnd,
            p,
            work: Arc::new(work),
            page: Page::Choose,
            ticked,
            launch: true,
            hot: None,
            press: None,
            focus: Hit::Primary,
            keys: false,
            dpi,
            pal: Pal::system(),
            fonts: Fonts { body: HFONT::default(), title: HFONT::default(), glyph: HFONT::default() },
            icon: HICON::default(),
        };
        ui.load_look();
        ui.tint_caption();
        UI.set(Some(ui));
        place(hwnd, dpi, style.0);
        let _ = ShowWindow(hwnd, if activate { SW_SHOW } else { SW_SHOWNOACTIVATE });
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        UI.set(None);
    }
}

fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|u| u.try_borrow_mut().ok().and_then(|mut u| u.as_mut().map(f)))
}

fn redraw(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let xy = || ((lp.0 & 0xFFFF) as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as i16 as i32);
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                let mut cr = RECT::default();
                let _ = GetClientRect(hwnd, &mut cr);
                let mem = CreateCompatibleDC(Some(dc));
                let bmp = CreateCompatibleBitmap(dc, cr.right, cr.bottom);
                let old = SelectObject(mem, bmp.into());
                with_ui(|u| u.paint(mem));
                let _ = BitBlt(dc, 0, 0, cr.right, cr.bottom, Some(mem), 0, 0, SRCCOPY);
                SelectObject(mem, old);
                let _ = DeleteObject(bmp.into());
                let _ = DeleteDC(mem);
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy();
                if with_ui(|u| {
                    let h = u.hit(x, y);
                    std::mem::replace(&mut u.hot, h) != h
                }) == Some(true)
                {
                    redraw(hwnd);
                }
                let mut t = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut t);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                with_ui(|u| u.hot = None);
                redraw(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy();
                with_ui(|u| {
                    u.press = u.hit(x, y);
                    if let Some(h) = u.press {
                        u.focus = h;
                    }
                    u.keys = false;
                });
                SetCapture(hwnd);
                redraw(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = xy();
                let _ = ReleaseCapture();
                let close = with_ui(|u| {
                    let h = u.press.take().filter(|&p| u.hit(x, y) == Some(p));
                    h.is_some_and(|h| u.activate(h))
                });
                redraw(hwnd);
                if close == Some(true) {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if with_ui(|u| u.key(wp.0 as u16)) == Some(true) {
                    let _ = DestroyWindow(hwnd);
                }
                redraw(hwnd);
                LRESULT(0)
            }
            WM_STEP => {
                let t = *Box::from_raw(lp.0 as *mut String);
                with_ui(|u| u.page = Page::Working { step: t, done: wp.0 as f32 / 1000.0 });
                redraw(hwnd);
                LRESULT(0)
            }
            WM_FINISHED => {
                let r = *Box::from_raw(lp.0 as *mut Result<PathBuf, String>);
                with_ui(|u| {
                    u.page = match r {
                        Ok(exe) => Page::Done(exe),
                        Err(e) => Page::Failed(e),
                    };
                    u.focus = Hit::Primary;
                });
                redraw(hwnd);
                LRESULT(0)
            }
            WM_DPICHANGED => {
                with_ui(|u| {
                    u.dpi = ((wp.0 >> 16) & 0xFFFF) as u32;
                    u.load_look();
                });
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                redraw(hwnd);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                // "ImmersiveColorSet": light or dark, or the accent, changed
                if lp.0 != 0 && PCWSTR(lp.0 as *const u16).to_string().is_ok_and(|s| s == "ImmersiveColorSet") {
                    with_ui(|u| {
                        u.pal = Pal::system();
                        u.tint_caption();
                    });
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                // not while the files are being written
                if with_ui(|u| !matches!(u.page, Page::Working { .. })) != Some(false) {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_in_order() {
        assert!(is_newer("0.2.10", "0.2.9"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.1"));
    }
}
