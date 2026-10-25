//! The candidate window and the mode hint, drawn with Direct2D in software (no
//! D3D device in the host app). Windows live per thread, like the text service.

use crate::{dll, settings::Settings};
use std::{cell::RefCell, mem::ManuallyDrop};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dwm::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::*,
        },
        System::Registry::*,
        UI::{HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
    },
    core::*,
};
use windows_numerics::Vector2;

const PAD: f32 = 10.0;
/// Between candidates: across in a row, down in a column.
const GAP: f32 = 14.0;
const ROW_GAP: f32 = 4.0;
/// Size of the numbers and the pinyin line, relative to the candidates.
const SMALL: f32 = 13.0 / 18.0;
const HINT: f32 = 36.0;

thread_local! {
    // Freed by `destroy` only: at process exit the D2D and window objects must
    // not be released, the system DLLs behind them may be gone already.
    static UI: RefCell<Option<ManuallyDrop<Ui>>> = const { RefCell::new(None) };
}

#[derive(Default)]
enum Scene {
    #[default]
    Empty,
    Cands { preedit: String, cands: Vec<String>, trad: bool, font: String, size: f32, vertical: bool },
    Hint(String),
}

#[derive(Default)]
struct Win {
    hwnd: HWND,
    rt: Option<ID2D1HwndRenderTarget>,
    scene: Scene,
}

struct Ui {
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    /// Text formats by (family, size bits, Traditional, bold).
    fmts: RefCell<Vec<((String, u32, bool, bool), IDWriteTextFormat)>>,
    cand: Win,
    hint: Win,
}

/// Text layouts with their position in DIPs and color (0 text, 1 dim, 2 on the
/// highlight), the highlight, and the window size.
struct Laid {
    items: Vec<(IDWriteTextLayout, f32, f32, u8)>,
    sel: Option<D2D_RECT_F>,
    w: f32,
    h: f32,
}

struct Theme {
    bg: D2D1_COLOR_F,
    text: D2D1_COLOR_F,
    dim: D2D1_COLOR_F,
    sel: D2D1_COLOR_F,
    sel_text: D2D1_COLOR_F,
}

fn rgb(c: u32) -> D2D1_COLOR_F {
    let f = |s: u32| ((c >> s) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F { r: f(16), g: f(8), b: f(0), a: 1.0 }
}

fn theme() -> Theme {
    if light("AppsUseLightTheme") {
        Theme { bg: rgb(0xF9F9F9), text: rgb(0x1A1A1A), dim: rgb(0x707070), sel: rgb(0x0067C0), sel_text: rgb(0xFFFFFF) }
    } else {
        Theme { bg: rgb(0x2C2C2C), text: rgb(0xF0F0F0), dim: rgb(0xA0A0A0), sel: rgb(0x4CC2FF), sel_text: rgb(0x000000) }
    }
}

/// Whether the Windows theme setting `name` is light (false if unreadable, e.g. sandboxed).
pub fn light(name: &str) -> bool {
    let (mut v, mut n) = (0u32, 4u32);
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            &HSTRING::from(name),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut v as *mut u32 as _),
            Some(&mut n),
        )
    };
    r == ERROR_SUCCESS && v != 0
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn with(f: impl FnOnce(&mut Ui) -> Result<()>) {
    UI.with(|u| {
        let Ok(mut u) = u.try_borrow_mut() else { return };
        if u.is_none() {
            *u = Ui::new().ok().map(ManuallyDrop::new);
        }
        if let Some(ui) = u.as_mut() {
            let _ = f(ui);
        }
    })
}

/// Shows the candidates under the text at `at` (screen coordinates), in the font,
/// size and layout of the settings.
pub fn cands(at: Option<RECT>, preedit: &str, cands: &[String], trad: bool, s: &Settings) {
    with(|ui| {
        let font = if trad { &s.font_trad } else { &s.font }.clone();
        ui.cand.scene = Scene::Cands { preedit: preedit.into(), cands: cands.to_vec(), trad, font, size: s.size as f32, vertical: s.vertical };
        ui.show(false, at)
    })
}

/// Shows a mode hint (中/英/繁/简/全/半) above the text at `at` for about a second.
pub fn hint(at: Option<RECT>, text: &str) {
    with(|ui| {
        ui.hint.scene = Scene::Hint(text.into());
        ui.show(true, at)?;
        unsafe { SetTimer(Some(ui.hint.hwnd), 1, 1000, None) };
        Ok(())
    })
}

pub fn hide() {
    with(|ui| unsafe { ShowWindow(ui.cand.hwnd, SW_HIDE).ok() });
}

pub fn hide_hint() {
    with(|ui| unsafe { ShowWindow(ui.hint.hwnd, SW_HIDE).ok() });
}

pub fn destroy() {
    UI.with(|u| {
        if let Some(ui) = u.try_borrow_mut().ok().and_then(|mut u| u.take()) {
            unsafe {
                let _ = DestroyWindow(ui.cand.hwnd);
                let _ = DestroyWindow(ui.hint.hwnd);
            }
            drop(ManuallyDrop::into_inner(ui));
        }
    })
}

extern "system" fn proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let r = std::panic::catch_unwind(|| unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(h, &mut ps);
                let _ = EndPaint(h, &ps);
                UI.with(|u| {
                    if let Some(ui) = u.try_borrow_mut().ok().as_deref_mut().and_then(|u| u.as_mut()) {
                        let hint = ui.hint.hwnd == h;
                        let _ = ui.render(hint);
                    }
                });
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = KillTimer(Some(h), w.0);
                let _ = ShowWindow(h, SW_HIDE);
                LRESULT(0)
            }
            _ => DefWindowProcW(h, msg, w, l),
        }
    });
    r.unwrap_or_default()
}

impl Ui {
    fn new() -> Result<Ui> {
        unsafe {
            let class = w!("BrushkeyCandidates");
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DROPSHADOW,
                lpfnWndProc: Some(proc),
                hInstance: dll().into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassExW(&wc); // fails harmlessly when another thread registered it
            let win = || -> Result<Win> {
                let hwnd = CreateWindowExW(
                    WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                    class,
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    None,
                    None,
                    Some(dll().into()),
                    None,
                )?;
                let corner = DWMWCP_ROUNDSMALL;
                let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, 4);
                Ok(Win { hwnd, ..Default::default() })
            };
            Ok(Ui {
                d2d: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
                dw: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                fmts: RefCell::new(vec![]),
                cand: win()?,
                hint: win()?,
            })
        }
    }

    /// Traditional text gets the Taiwan locale, so fonts covering several regions draw Taiwan glyph forms.
    fn format(&self, family: &str, size: f32, trad: bool, bold: bool) -> Result<IDWriteTextFormat> {
        let k = (family.to_string(), size.to_bits(), trad, bold);
        if let Some((_, f)) = self.fmts.borrow().iter().find(|(x, _)| *x == k) {
            return Ok(f.clone());
        }
        let weight = if bold { DWRITE_FONT_WEIGHT_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
        let locale = if trad { w!("zh-TW") } else { w!("zh-CN") };
        let f = unsafe { self.dw.CreateTextFormat(&HSTRING::from(family), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, locale)? };
        self.fmts.borrow_mut().push((k, f.clone()));
        Ok(f)
    }

    fn layout(&self, s: &str, f: &IDWriteTextFormat) -> Result<(IDWriteTextLayout, f32, f32)> {
        unsafe {
            let l = self.dw.CreateTextLayout(&wide(s), f, 4096.0, 4096.0)?;
            let mut m = DWRITE_TEXT_METRICS::default();
            l.GetMetrics(&mut m)?;
            Ok((l, m.widthIncludingTrailingWhitespace, m.height))
        }
    }

    /// Lays out the candidate (or hint) window.
    fn lay(&self, hint: bool) -> Result<Laid> {
        let win = if hint { &self.hint } else { &self.cand };
        let mut items = vec![];
        match &win.scene {
            Scene::Empty => Ok(Laid { items, sel: None, w: 1.0, h: 1.0 }),
            Scene::Hint(t) => {
                let trad = t == "繁";
                let family = if trad { "Microsoft JhengHei UI" } else { "Microsoft YaHei UI" };
                let (l, w, h) = self.layout(t, &self.format(family, 20.0, trad, true)?)?;
                items.push((l, (HINT - w) / 2.0, (HINT - h) / 2.0, 0));
                Ok(Laid { items, sel: None, w: HINT, h: HINT })
            }
            Scene::Cands { preedit, cands, trad, font, size, vertical } => {
                let (big, small) = (self.format(font, *size, *trad, false)?, self.format(font, size * SMALL, *trad, false)?);
                let (l, pw, ph) = self.layout(preedit, &small)?;
                items.push((l, PAD, PAD - 2.0, 0));
                let top = PAD + ph;
                // (x, y): where the next candidate goes; (right, bottom): the extent so far
                let (mut x, mut y, mut right, mut bottom, mut sel) = (PAD, top, PAD + pw, top, None);
                for (i, c) in cands.iter().enumerate() {
                    let (nl, nw, nh) = self.layout(&format!("{}", i + 1), &small)?;
                    let (tl, tw, th) = self.layout(c, &big)?;
                    let first = i == 0;
                    items.push((nl, x, y + (th - nh) / 2.0 + 1.0, if first { 2 } else { 1 }));
                    items.push((tl, x + nw + 3.0, y, if first { 2 } else { 0 }));
                    let end = x + nw + 3.0 + tw;
                    if first {
                        sel = Some(D2D_RECT_F { left: x - 5.0, top: y - 1.0, right: end + 5.0, bottom: y + th + 1.0 });
                    }
                    (right, bottom) = (right.max(end), bottom.max(y + th));
                    if *vertical { y += th + ROW_GAP } else { x = end + GAP }
                }
                // in a column the highlight spans the width
                if let (true, Some(r)) = (*vertical, &mut sel) {
                    r.right = right + 5.0;
                }
                Ok(Laid { items, sel, w: right + PAD, h: bottom + PAD })
            }
        }
    }

    /// Positions the window next to `at`, then draws it.
    fn show(&mut self, hint: bool, at: Option<RECT>) -> Result<()> {
        let laid = self.lay(hint)?;
        let (w, h) = (laid.w, laid.h);
        let win = if hint { &mut self.hint } else { &mut self.cand };
        unsafe {
            let s = GetDpiForWindow(win.hwnd).max(96) as f32 / 96.0;
            let (pw, ph) = ((w * s).ceil() as i32, (h * s).ceil() as i32);
            let at = at.unwrap_or_else(|| {
                let mut p = POINT::default();
                let _ = GetCursorPos(&mut p);
                RECT { left: p.x, top: p.y, right: p.x, bottom: p.y + 20 }
            });
            let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            let _ = GetMonitorInfoW(MonitorFromRect(&at, MONITOR_DEFAULTTONEAREST), &mut mi);
            let work = mi.rcWork;
            let gap = (4.0 * s) as i32;
            let (below, above) = (at.bottom + gap, at.top - gap - ph);
            // Candidates go below the text, the hint above it; each flips when there's no room.
            let y = match hint {
                false if below + ph > work.bottom => above,
                true if above < work.top => below,
                false => below,
                true => above,
            };
            let x = at.left.min(work.right - pw).max(work.left);
            SetWindowPos(win.hwnd, Some(HWND_TOPMOST), x, y, pw, ph, SWP_NOACTIVATE | SWP_SHOWWINDOW)?;
            let size = D2D_SIZE_U { width: pw as u32, height: ph as u32 };
            match &win.rt {
                Some(rt) => rt.Resize(&size)?,
                None => {
                    let props = D2D1_RENDER_TARGET_PROPERTIES {
                        r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                        pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_IGNORE },
                        ..Default::default()
                    };
                    let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES { hwnd: win.hwnd, pixelSize: size, presentOptions: D2D1_PRESENT_OPTIONS_NONE };
                    win.rt = Some(self.d2d.CreateHwndRenderTarget(&props, &hp)?);
                }
            }
            win.rt.as_ref().unwrap().SetDpi(96.0 * s, 96.0 * s);
        }
        self.draw(hint, laid)
    }

    fn render(&mut self, hint: bool) -> Result<()> {
        let laid = self.lay(hint)?;
        self.draw(hint, laid)
    }

    fn draw(&mut self, hint: bool, Laid { items, sel, .. }: Laid) -> Result<()> {
        let win = if hint { &mut self.hint } else { &mut self.cand };
        let Some(rt) = win.rt.clone() else { return Ok(()) };
        let t = theme();
        unsafe {
            rt.BeginDraw();
            rt.Clear(Some(&t.bg));
            let brush = rt.CreateSolidColorBrush(&t.sel, None)?;
            if let Some(r) = sel {
                rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: 4.0, radiusY: 4.0 }, &brush);
            }
            for (l, x, y, c) in &items {
                brush.SetColor(&[t.text, t.dim, t.sel_text][*c as usize]);
                rt.DrawTextLayout(Vector2 { X: *x, Y: *y }, l, &brush, D2D1_DRAW_TEXT_OPTIONS_NONE);
            }
            if rt.EndDraw(None, None).is_err() {
                win.rt = None; // device lost; recreated on the next show
            }
        }
        Ok(())
    }
}
