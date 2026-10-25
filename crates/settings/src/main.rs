//! Brushkey settings: one self-drawn window (Direct2D) over the settings file.
//! Every change is saved at once; the IME in each app reads it on its next focus.

#![windows_subsystem = "windows"]

use brushkey::{
    light,
    settings::{Settings, Toggle},
    user_dir,
};
use engine::FUZZY;
use std::cell::RefCell;
use windows::{
    Win32::{
        Foundation::*,
        Globalization::GetUserDefaultUILanguage,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dwm::*,
            Gdi::*,
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::*,
            Input::{Ime::ImmDisableIME, KeyboardAndMouse::*},
            Shell::ShellExecuteW,
            WindowsAndMessaging::*,
        },
    },
    core::*,
};
use windows_numerics::Vector2;

const CLASS: PCWSTR = w!("BrushkeySettings");
/// Not in the `windows` modules used here (it lives with the common controls).
const WM_MOUSELEAVE: u32 = 0x02A3;

// Layout in DIPs: two columns of cards.
const W: f32 = 760.0;
const H: f32 = 392.0;
const M: f32 = 20.0;
const COL: f32 = 352.0;
const TITLE: f32 = 28.0;
const ROW: f32 = 40.0;
const PAD: f32 = 14.0;
const CTRL: f32 = 28.0;
/// Fuzzy pairs: rows of three.
const FUZZY_ROW: f32 = 34.0;
/// The open font list: row height, width, most rows shown.
const ITEM: f32 = 28.0;
const LIST_W: f32 = 240.0;
const LIST_ROWS: usize = 10;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Toggle(Toggle),
    English(bool),
    Vertical(bool),
    /// -1 / +1 steps.
    Page(i32),
    Size(i32),
    /// Opens the font list: 0 Simplified, 1 Traditional.
    Font(usize),
    /// A row of the open font list.
    Pick(usize),
    Fuzzy(usize),
    Clear,
    GitHub,
}

struct Font {
    /// The English family name, stored in the settings.
    name: String,
    /// The name in the UI language.
    label: String,
}

struct Theme {
    bg: u32,
    card: u32,
    line: u32,
    text: u32,
    dim: u32,
    accent: u32,
    on_accent: u32,
    ctrl: u32,
    ctrl_line: u32,
    hover: u32,
}

fn theme() -> Theme {
    if light("AppsUseLightTheme") {
        Theme { bg: 0xF3F3F3, card: 0xFFFFFF, line: 0xE5E5E5, text: 0x1A1A1A, dim: 0x6B6B6B, accent: 0x0067C0, on_accent: 0xFFFFFF, ctrl: 0xFBFBFB, ctrl_line: 0xD1D1D1, hover: 0xEAEAEA }
    } else {
        Theme { bg: 0x202020, card: 0x2B2B2B, line: 0x1D1D1D, text: 0xFFFFFF, dim: 0xA8A8A8, accent: 0x4CC2FF, on_accent: 0x000000, ctrl: 0x373737, ctrl_line: 0x454545, hover: 0x454545 }
    }
}

fn rgb(c: u32) -> D2D1_COLOR_F {
    let f = |s: u32| ((c >> s) & 0xFF) as f32 / 255.0;
    D2D1_COLOR_F { r: f(16), g: f(8), b: f(0), a: 1.0 }
}

fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

fn inset(r: D2D_RECT_F, d: f32) -> D2D_RECT_F {
    rect(r.left + d, r.top + d, r.right - d, r.bottom - d)
}

fn inside(r: &D2D_RECT_F, p: Vector2) -> bool {
    (r.left..r.right).contains(&p.X) && (r.top..r.bottom).contains(&p.Y)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// System fonts that have Simplified (们这) and Traditional (們這) glyphs, by English name.
fn fonts(dw: &IDWriteFactory, zh: bool) -> Result<[Vec<Font>; 2]> {
    let mut lists: [Vec<Font>; 2] = Default::default();
    unsafe {
        let mut c = None;
        dw.GetSystemFontCollection(&mut c, false)?;
        let c = c.ok_or(Error::from(E_FAIL))?;
        for i in 0..c.GetFontFamilyCount() {
            let fam = c.GetFontFamily(i)?;
            let names = fam.GetFamilyNames()?;
            // the name in `locale`; None: the first name, whatever its locale
            let name = |locale: Option<PCWSTR>| -> Option<String> {
                let idx = match locale {
                    None => 0,
                    Some(l) => {
                        let (mut idx, mut found) = (0, FALSE);
                        names.FindLocaleName(l, &mut idx, &mut found).ok()?;
                        found.as_bool().then_some(idx)?
                    }
                };
                let mut buf = vec![0u16; names.GetStringLength(idx).ok()? as usize + 1];
                names.GetString(idx, &mut buf).ok()?;
                Some(String::from_utf16_lossy(&buf[..buf.len() - 1]))
            };
            let Some(en) = name(Some(w!("en-us"))).or_else(|| name(None)) else { continue };
            let label = if zh { name(Some(w!("zh-cn"))).or_else(|| name(Some(w!("zh-tw")))) } else { None }.unwrap_or_else(|| en.clone());
            let font = fam.GetFirstMatchingFont(DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL)?;
            let has = |s: &str| s.chars().all(|ch| font.HasCharacter(ch as u32).is_ok_and(|b| b.as_bool()));
            for (list, chars) in lists.iter_mut().zip(["们这", "們這"]) {
                if has(chars) {
                    list.push(Font { name: en.clone(), label: label.clone() });
                }
            }
        }
    }
    for l in &mut lists {
        l.sort_by(|a, b| a.name.cmp(&b.name));
    }
    Ok(lists)
}

struct App {
    hwnd: HWND,
    s: Settings,
    zh: bool,
    fonts: [Vec<Font>; 2],
    /// The open font list and its first row shown.
    open: Option<usize>,
    scroll: usize,
    mouse: Option<Vector2>,
    hover: Option<Hit>,
    /// Clickable areas of the last frame in drawing order, and the two font boxes.
    hits: Vec<(D2D_RECT_F, Hit)>,
    anchors: [D2D_RECT_F; 2],
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    /// Text formats by (family, size bits, bold).
    fmts: RefCell<Vec<((String, u32, bool), IDWriteTextFormat)>>,
    rt: Option<ID2D1HwndRenderTarget>,
}

impl App {
    fn new(hwnd: HWND) -> Result<App> {
        let s = Settings::load();
        // the Windows display language decides until one is picked here
        let zh = s.english.map_or_else(|| unsafe { GetUserDefaultUILanguage() } & 0x3FF == 0x04, |e| !e);
        let dw: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        Ok(App {
            hwnd,
            fonts: fonts(&dw, zh)?,
            s,
            zh,
            open: None,
            scroll: 0,
            mouse: None,
            hover: None,
            hits: vec![],
            anchors: Default::default(),
            d2d: unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? },
            dw,
            fmts: RefCell::new(vec![]),
            rt: None,
        })
    }

    fn tr(&self, zh: &'static str, en: &'static str) -> &'static str {
        if self.zh { zh } else { en }
    }

    fn title(&self) -> &'static str {
        self.tr("Brushkey 拼音设置", "Brushkey Settings")
    }

    fn scale(&self) -> f32 {
        unsafe { GetDpiForWindow(self.hwnd).max(96) as f32 / 96.0 }
    }

    fn font(&self, t: usize) -> &str {
        if t == 0 { &self.s.font } else { &self.s.font_trad }
    }

    /// The topmost clickable area under `p`.
    fn hit_at(&self, p: Option<Vector2>) -> Option<Hit> {
        let p = p?;
        self.hits.iter().rev().find(|(r, _)| inside(r, p)).map(|(_, h)| *h)
    }

    /// Where the list of font box `t` opens (below it, or above when that has more room) and how many rows it shows.
    fn list_box(&self, t: usize) -> (D2D_RECT_F, usize) {
        let a = self.anchors[t];
        let (below, above) = (H - a.bottom - 12.0, a.top - 12.0);
        let rows = self.fonts[t].len().min(((below.max(above) - 8.0) / ITEM) as usize).clamp(1, LIST_ROWS);
        let h = rows as f32 * ITEM + 8.0;
        let top = if below >= h || below >= above { a.bottom + 4.0 } else { a.top - 4.0 - h };
        (rect(a.right - LIST_W, top, a.right, top + h), rows)
    }

    /// Applies a click; returns true when it asks to clear the memory (confirmed outside, see `wndproc`).
    fn click(&mut self, hit: Option<Hit>) -> bool {
        let before = self.s.clone();
        let open = self.open.take();
        match hit {
            // with the font list open, a click elsewhere only closes it
            Some(Hit::Pick(i)) => {
                if let Some((t, name)) = open.and_then(|t| self.fonts[t].get(i).map(|f| (t, f.name.clone()))) {
                    *if t == 0 { &mut self.s.font } else { &mut self.s.font_trad } = name;
                }
            }
            _ if open.is_some() => {}
            Some(Hit::Font(t)) if !self.fonts[t].is_empty() => {
                let cur = self.fonts[t].iter().position(|f| f.name == self.font(t)).unwrap_or(0);
                (self.open, self.scroll) = (Some(t), cur.saturating_sub(LIST_ROWS / 2));
            }
            Some(Hit::Toggle(k)) => self.s.toggle = k,
            Some(Hit::English(e)) => {
                self.s.english = Some(e);
                if self.zh == e {
                    self.zh = !e;
                    self.fonts = fonts(&self.dw, self.zh).unwrap_or_default();
                    unsafe {
                        let _ = SetWindowTextW(self.hwnd, &HSTRING::from(self.title()));
                    }
                }
            }
            Some(Hit::Vertical(v)) => self.s.vertical = v,
            Some(Hit::Page(d)) => self.s.page = (self.s.page as i32 + d).clamp(3, 9) as usize,
            Some(Hit::Size(d)) => self.s.size = (self.s.size as i32 + 2 * d).clamp(12, 36) as u32,
            Some(Hit::Fuzzy(i)) => self.s.fuzzy ^= 1 << i,
            Some(Hit::Clear) => return true,
            Some(Hit::GitHub) => unsafe {
                ShellExecuteW(Some(self.hwnd), w!("open"), w!("https://github.com/ArthurLauCS/Brushkey"), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
            },
            _ => {}
        }
        if self.s != before {
            self.s.save();
        }
        self.redraw();
        false
    }

    fn wheel(&mut self, notches: i32) {
        let Some(t) = self.open else { return };
        let (_, rows) = self.list_box(t);
        let max = self.fonts[t].len().saturating_sub(rows);
        self.scroll = (self.scroll.min(max) as i32 - notches).clamp(0, max as i32) as usize;
        self.hover = None;
        self.redraw();
    }

    fn moved(&mut self, p: Option<Vector2>) {
        self.mouse = p;
        let h = self.hit_at(p);
        if h != self.hover {
            self.hover = h;
            self.redraw();
        }
    }

    fn redraw(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Dark title bar in dark mode.
    fn caption(&self) {
        let dark = BOOL::from(!light("AppsUseLightTheme"));
        unsafe {
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        }
    }

    fn resize(&self) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut r);
            if let Some(rt) = &self.rt {
                let _ = rt.Resize(&D2D_SIZE_U { width: r.right as u32, height: r.bottom as u32 });
            }
        }
    }

    fn paint(&mut self) -> Result<()> {
        unsafe {
            if self.rt.is_none() {
                let mut r = RECT::default();
                GetClientRect(self.hwnd, &mut r)?;
                let hp = D2D1_HWND_RENDER_TARGET_PROPERTIES { hwnd: self.hwnd, pixelSize: D2D_SIZE_U { width: r.right as u32, height: r.bottom as u32 }, presentOptions: D2D1_PRESENT_OPTIONS_NONE };
                self.rt = Some(self.d2d.CreateHwndRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES::default(), &hp)?);
            }
            let rt = self.rt.clone().unwrap();
            let s = self.scale();
            rt.SetDpi(96.0 * s, 96.0 * s);
            let t = theme();
            rt.BeginDraw();
            rt.Clear(Some(&rgb(t.bg)));
            let mut pen = Pen { app: self, brush: rt.CreateSolidColorBrush(&rgb(t.text), None)?, rt: &rt, t, hits: vec![], anchors: Default::default() };
            pen.page();
            let (hits, anchors) = (pen.hits, pen.anchors);
            (self.hits, self.anchors) = (hits, anchors);
            if rt.EndDraw(None, None).is_err() {
                self.rt = None; // device lost; recreated on the next paint
            }
        }
        Ok(())
    }
}

/// One frame's drawing.
struct Pen<'a> {
    app: &'a App,
    rt: &'a ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    t: Theme,
    hits: Vec<(D2D_RECT_F, Hit)>,
    anchors: [D2D_RECT_F; 2],
}

impl Pen<'_> {
    fn page(&mut self) {
        let a = self.app;
        let (s, tr) = (&a.s, |zh, en| a.tr(zh, en));
        let lines = [Toggle::Shift, Toggle::Ctrl, Toggle::Off].map(|k| (["Shift", "Ctrl", tr("关", "Off")][k as usize], Hit::Toggle(k), s.toggle == k));

        // left: general, candidate window
        let (x0, x1) = (M, M + COL);
        let mut y = M;
        let cy = |y: f32, row: usize| y + ROW * (row as f32 + 0.5);
        self.title(x0, y, tr("常规", "General"));
        y += TITLE;
        self.card(x0, y, x1, 2);
        self.label(x0 + PAD, cy(y, 0), tr("中/英切换键", "Chinese/English key"));
        self.choice(x1 - PAD, cy(y, 0), &lines);
        self.label(x0 + PAD, cy(y, 1), tr("语言/Language", "Language"));
        self.choice(x1 - PAD, cy(y, 1), &[("中文/Chinese", Hit::English(false), a.zh), ("English", Hit::English(true), !a.zh)]);
        y += 2.0 * ROW + 16.0;
        self.title(x0, y, tr("候选窗", "Candidate window"));
        y += TITLE;
        self.card(x0, y, x1, 5);
        self.label(x0 + PAD, cy(y, 0), tr("排列", "Layout"));
        self.choice(x1 - PAD, cy(y, 0), &[(tr("横排", "Horizontal"), Hit::Vertical(false), !s.vertical), (tr("竖排", "Vertical"), Hit::Vertical(true), s.vertical)]);
        self.label(x0 + PAD, cy(y, 1), tr("每页个数", "Per page"));
        self.stepper(x1 - PAD, cy(y, 1), s.page as u32, Hit::Page(-1), Hit::Page(1), s.page <= 3, s.page >= 9);
        self.label(x0 + PAD, cy(y, 2), tr("字号", "Size"));
        self.stepper(x1 - PAD, cy(y, 2), s.size, Hit::Size(-1), Hit::Size(1), s.size <= 12, s.size >= 36);
        for t in 0..2 {
            self.label(x0 + PAD, cy(y, 3 + t), [tr("简体字体", "Simplified font"), tr("繁体字体", "Traditional font")][t]);
            self.anchors[t] = self.dropdown(x1 - PAD, cy(y, 3 + t), t);
        }

        // right: fuzzy pinyin, memory, version
        let (x0, x1) = (M + COL + 16.0, W - M);
        let mut y = M;
        self.title(x0, y, tr("模糊音", "Fuzzy pinyin"));
        y += TITLE;
        let rows = FUZZY.len().div_ceil(3);
        self.card_h(x0, y, x1, rows as f32 * FUZZY_ROW + 12.0);
        let col = (x1 - x0 - 2.0 * PAD) / 3.0;
        for (i, (p, q)) in FUZZY.iter().enumerate() {
            let (x, fy) = (x0 + PAD + col * (i % 3) as f32, y + 6.0 + FUZZY_ROW * ((i / 3) as f32 + 0.5));
            self.check(x, fy, &format!("{p} = {q}"), s.fuzzy >> i & 1 == 1, Hit::Fuzzy(i));
        }
        y += rows as f32 * FUZZY_ROW + 12.0 + 16.0;
        self.title(x0, y, tr("记忆", "Memory"));
        y += TITLE;
        self.card(x0, y, x1, 1);
        self.label(x0 + PAD, cy(y, 0), tr("学过的词和词序", "Learned words and order"));
        self.button(x1 - PAD, cy(y, 0), tr("清空记忆", "Clear memory"), Hit::Clear);
        let (dim, fy) = (self.t.dim, H - M - 8.0);
        let w = self.text(&format!("Brushkey {}", env!("CARGO_PKG_VERSION")), x1, fy, 12.0, false, dim, None, 1.0);
        let gx = x1 - w - 16.0;
        let gw = self.text("GitHub", gx, fy, 12.0, false, if self.hot(Hit::GitHub) { self.t.accent } else { dim }, None, 1.0);
        self.hits.push((rect(gx - gw - 4.0, fy - 10.0, gx + 4.0, fy + 10.0), Hit::GitHub));

        if let Some(t) = a.open {
            self.list(t);
        }
    }

    fn color(&self, c: u32) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&rgb(c)) };
        &self.brush
    }

    fn fill(&self, r: D2D_RECT_F, radius: f32, c: u32) {
        unsafe { self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, self.color(c)) };
    }

    fn outline(&self, r: D2D_RECT_F, radius: f32, c: u32) {
        unsafe { self.rt.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: inset(r, 0.5), radiusX: radius, radiusY: radius }, self.color(c), 1.0, None) };
    }

    fn line(&self, a: (f32, f32), b: (f32, f32), width: f32, c: u32) {
        unsafe { self.rt.DrawLine(Vector2 { X: a.0, Y: a.1 }, Vector2 { X: b.0, Y: b.1 }, self.color(c), width, None) };
    }

    /// `family` None: the UI font. The zh-CN locale makes Chinese in the English UI fall back to a Chinese font.
    fn layout(&self, s: &str, size: f32, bold: bool, family: Option<&str>) -> Option<(IDWriteTextLayout, f32, f32)> {
        let family = family.unwrap_or(if self.app.zh { "Microsoft YaHei UI" } else { "Segoe UI" });
        let k = (family.to_string(), size.to_bits(), bold);
        let cached = self.app.fmts.borrow().iter().find(|(x, _)| *x == k).map(|(_, f)| f.clone());
        unsafe {
            let f = match cached {
                Some(f) => f,
                None => {
                    let weight = if bold { DWRITE_FONT_WEIGHT_SEMI_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
                    let f = self.app.dw.CreateTextFormat(&HSTRING::from(family), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("zh-CN")).ok()?;
                    self.app.fmts.borrow_mut().push((k, f.clone()));
                    f
                }
            };
            let l = self.app.dw.CreateTextLayout(&wide(s), &f, 4096.0, 4096.0).ok()?;
            let mut m = DWRITE_TEXT_METRICS::default();
            l.GetMetrics(&mut m).ok()?;
            Some((l, m.widthIncludingTrailingWhitespace, m.height))
        }
    }

    fn width(&self, s: &str) -> f32 {
        self.layout(s, 14.0, false, None).map_or(0.0, |l| l.1)
    }

    /// Draws `s` centered on `cy`; `ax` places it: 0 left edge at `x`, 0.5 centered, 1 right edge. Returns its width.
    #[allow(clippy::too_many_arguments)]
    fn text(&self, s: &str, x: f32, cy: f32, size: f32, bold: bool, c: u32, family: Option<&str>, ax: f32) -> f32 {
        let Some((l, w, h)) = self.layout(s, size, bold, family) else { return 0.0 };
        unsafe { self.rt.DrawTextLayout(Vector2 { X: x - w * ax, Y: cy - h / 2.0 }, &l, self.color(c), D2D1_DRAW_TEXT_OPTIONS_NONE) };
        w
    }

    fn label(&self, x: f32, cy: f32, s: &str) {
        self.text(s, x, cy, 14.0, false, self.t.text, None, 0.0);
    }

    fn title(&self, x: f32, y: f32, s: &str) {
        self.text(s, x + 2.0, y + TITLE / 2.0 - 2.0, 14.0, true, self.t.text, None, 0.0);
    }

    fn hot(&self, h: Hit) -> bool {
        self.app.hover == Some(h)
    }

    /// A card of `rows` rows with dividers between them.
    fn card(&self, x0: f32, y: f32, x1: f32, rows: usize) {
        self.card_h(x0, y, x1, rows as f32 * ROW);
        for i in 1..rows {
            let ly = y + ROW * i as f32;
            self.line((x0 + PAD, ly), (x1 - PAD, ly), 1.0, self.t.line);
        }
    }

    fn card_h(&self, x0: f32, y: f32, x1: f32, h: f32) {
        let r = rect(x0, y, x1, y + h);
        self.fill(r, 6.0, self.t.card);
        self.outline(r, 6.0, self.t.line);
    }

    /// Segmented choice ending at `right`: (label, hit, selected).
    fn choice(&mut self, right: f32, cy: f32, opts: &[(&str, Hit, bool)]) {
        let ws: Vec<f32> = opts.iter().map(|o| self.width(o.0) + 24.0).collect();
        let mut x = right - ws.iter().sum::<f32>();
        let r = rect(x, cy - CTRL / 2.0, right, cy + CTRL / 2.0);
        self.fill(r, 4.0, self.t.ctrl);
        self.outline(r, 4.0, self.t.ctrl_line);
        for (&(label, hit, on), w) in opts.iter().zip(ws) {
            let seg = rect(x, r.top, x + w, r.bottom);
            if on || self.hot(hit) {
                self.fill(inset(seg, 3.0), 3.0, if on { self.t.accent } else { self.t.hover });
            }
            self.text(label, x + w / 2.0, cy, 14.0, false, if on { self.t.on_accent } else { self.t.text }, None, 0.5);
            self.hits.push((seg, hit));
            x += w;
        }
    }

    /// [−] value [+] ending at `right`; a button at its limit is grayed and does nothing.
    #[allow(clippy::too_many_arguments)]
    fn stepper(&mut self, right: f32, cy: f32, value: u32, dec: Hit, inc: Hit, at_min: bool, at_max: bool) {
        let r = rect(right - 100.0, cy - CTRL / 2.0, right, cy + CTRL / 2.0);
        self.fill(r, 4.0, self.t.ctrl);
        self.outline(r, 4.0, self.t.ctrl_line);
        for (i, (hit, off)) in [(dec, at_min), (inc, at_max)].into_iter().enumerate() {
            let b = if i == 0 { rect(r.left, r.top, r.left + 32.0, r.bottom) } else { rect(r.right - 32.0, r.top, r.right, r.bottom) };
            if !off && self.hot(hit) {
                self.fill(inset(b, 3.0), 3.0, self.t.hover);
            }
            let c = if off { self.t.ctrl_line } else { self.t.text };
            let (mx, my) = ((b.left + b.right) / 2.0, cy);
            self.line((mx - 5.0, my), (mx + 5.0, my), 1.2, c);
            if i == 1 {
                self.line((mx, my - 5.0), (mx, my + 5.0), 1.2, c);
            }
            if !off {
                self.hits.push((b, hit));
            }
        }
        self.text(&value.to_string(), (r.left + r.right) / 2.0, cy, 14.0, false, self.t.text, None, 0.5);
    }

    /// The font box `t`, showing the font's name in the font itself.
    fn dropdown(&mut self, right: f32, cy: f32, t: usize) -> D2D_RECT_F {
        let a = self.app;
        let r = rect(right - 196.0, cy - CTRL / 2.0, right, cy + CTRL / 2.0);
        let hit = Hit::Font(t);
        self.fill(r, 4.0, if self.hot(hit) || a.open == Some(t) { self.t.hover } else { self.t.ctrl });
        self.outline(r, 4.0, self.t.ctrl_line);
        let name = a.font(t);
        let label = a.fonts[t].iter().find(|f| f.name == name).map_or(name, |f| f.label.as_str());
        unsafe { self.rt.PushAxisAlignedClip(&rect(r.left, r.top, r.right - 26.0, r.bottom), D2D1_ANTIALIAS_MODE_ALIASED) };
        self.text(label, r.left + 10.0, cy, 14.0, false, self.t.text, Some(name), 0.0);
        unsafe { self.rt.PopAxisAlignedClip() };
        let x = r.right - 15.0;
        self.line((x - 4.0, cy - 2.0), (x, cy + 2.0), 1.2, self.t.dim);
        self.line((x, cy + 2.0), (x + 4.0, cy - 2.0), 1.2, self.t.dim);
        self.hits.push((r, hit));
        r
    }

    fn check(&mut self, x: f32, cy: f32, label: &str, on: bool, hit: Hit) {
        let b = rect(x, cy - 9.0, x + 18.0, cy + 9.0);
        if on {
            self.fill(b, 3.0, self.t.accent);
            let c = self.t.on_accent;
            self.line((x + 4.0, cy), (x + 7.5, cy + 3.5), 1.6, c);
            self.line((x + 7.5, cy + 3.5), (x + 14.0, cy - 3.5), 1.6, c);
        } else {
            self.fill(b, 3.0, if self.hot(hit) { self.t.hover } else { self.t.ctrl });
            self.outline(b, 3.0, self.t.dim);
        }
        let w = self.text(label, x + 26.0, cy, 14.0, false, self.t.text, None, 0.0);
        self.hits.push((rect(x - 4.0, cy - 14.0, x + 30.0 + w, cy + 14.0), hit));
    }

    fn button(&mut self, right: f32, cy: f32, label: &str, hit: Hit) {
        let w = self.width(label) + 32.0;
        let r = rect(right - w, cy - CTRL / 2.0, right, cy + CTRL / 2.0);
        self.fill(r, 4.0, if self.hot(hit) { self.t.hover } else { self.t.ctrl });
        self.outline(r, 4.0, self.t.ctrl_line);
        self.text(label, right - w / 2.0, cy, 14.0, false, self.t.text, None, 0.5);
        self.hits.push((r, hit));
    }

    /// The open font list: each name in its own font, the current one marked.
    fn list(&mut self, t: usize) {
        let a = self.app;
        let fonts = &a.fonts[t];
        let (r, rows) = a.list_box(t);
        let top = a.scroll.min(fonts.len() - rows);
        self.fill(r, 6.0, self.t.card);
        self.outline(r, 6.0, self.t.ctrl_line);
        unsafe { self.rt.PushAxisAlignedClip(&inset(r, 1.0), D2D1_ANTIALIAS_MODE_ALIASED) };
        for (row, (i, f)) in fonts.iter().enumerate().skip(top).take(rows).enumerate() {
            let y = r.top + 4.0 + ITEM * row as f32;
            let item = rect(r.left + 4.0, y, r.right - 4.0, y + ITEM);
            if self.hot(Hit::Pick(i)) {
                self.fill(item, 4.0, self.t.hover);
            }
            if f.name == a.font(t) {
                self.fill(rect(item.left, y + 8.0, item.left + 3.0, y + ITEM - 8.0), 1.5, self.t.accent);
            }
            self.text(&f.label, item.left + 12.0, y + ITEM / 2.0, 14.0, false, self.t.text, Some(&f.name), 0.0);
            self.hits.push((item, Hit::Pick(i)));
        }
        if fonts.len() > rows {
            let span = r.bottom - r.top - 8.0;
            let (h, y) = (span * rows as f32 / fonts.len() as f32, r.top + 4.0 + span * top as f32 / fonts.len() as f32);
            self.fill(rect(r.right - 6.0, y, r.right - 3.0, y + h), 1.5, self.t.dim);
        }
        unsafe { self.rt.PopAxisAlignedClip() };
    }
}

fn with<T>(f: impl FnOnce(&mut App) -> T) -> Option<T> {
    APP.with(|a| a.try_borrow_mut().ok()?.as_mut().map(f))
}

/// Mouse position from a message, in DIPs.
fn point(app: &App, l: LPARAM) -> Vector2 {
    let s = app.scale();
    Vector2 { X: (l.0 & 0xFFFF) as i16 as f32 / s, Y: ((l.0 >> 16) & 0xFFFF) as i16 as f32 / s }
}

extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let r = std::panic::catch_unwind(|| unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(h, &mut ps);
                let _ = EndPaint(h, &ps);
                let _ = with(|a| a.paint());
            }
            WM_ERASEBKGND => return LRESULT(1),
            WM_SIZE => {
                with(|a| a.resize());
            }
            WM_DPICHANGED => {
                let r = &*(l.0 as *const RECT);
                let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            WM_MOUSEMOVE => {
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut tme);
                with(|a| a.moved(Some(point(a, l))));
            }
            WM_MOUSELEAVE => {
                with(|a| a.moved(None));
            }
            WM_LBUTTONUP => {
                let clear = with(|a| a.click(a.hit_at(Some(point(a, l))))) == Some(true);
                if clear {
                    // asked outside `with`: the dialog's message loop paints this window meanwhile
                    let (title, text) = with(|a| (a.title(), a.tr("清空所有学过的词和词序？清空后不能恢复。", "Forget all learned words and their order? This cannot be undone."))).unwrap();
                    if MessageBoxW(Some(h), &HSTRING::from(text), &HSTRING::from(title), MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2) == IDOK {
                        // every app's IME sees the emptied log and starts over
                        if let Some(d) = user_dir() {
                            let _ = std::fs::write(d.join("user.log"), "");
                        }
                    }
                }
            }
            WM_MOUSEWHEEL => {
                with(|a| a.wheel((w.0 >> 16) as i16 as i32 / 120));
            }
            WM_KEYDOWN if w.0 == VK_ESCAPE.0 as usize => {
                // Esc closes the font list, or the window
                if with(|a| a.open.take().map(|_| a.redraw())).flatten().is_none() {
                    let _ = DestroyWindow(h);
                }
            }
            WM_SETTINGCHANGE => {
                with(|a| {
                    a.caption();
                    a.redraw()
                });
            }
            WM_DESTROY => PostQuitMessage(0),
            _ => return DefWindowProcW(h, msg, w, l),
        }
        LRESULT(0)
    });
    r.unwrap_or_default()
}

fn main() -> Result<()> {
    unsafe {
        // one window: a second start brings the first one forward
        if let Ok(h) = FindWindowW(CLASS, PCWSTR::null()) {
            if IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            let _ = SetForegroundWindow(h);
            return Ok(());
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // no text input here, so no input method gets loaded
        let _ = ImmDisableIME(u32::MAX);
        let inst = GetModuleHandleW(None)?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            // the app icon (resource 1); Windows picks the small size from it
            hIcon: LoadIconW(Some(inst.into()), PCWSTR(1 as _)).unwrap_or_default(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let hwnd = CreateWindowExW(WINDOW_EX_STYLE::default(), CLASS, w!(""), style, CW_USEDEFAULT, CW_USEDEFAULT, 100, 100, None, None, Some(inst.into()), None)?;
        let app = App::new(hwnd)?;
        SetWindowTextW(hwnd, &HSTRING::from(app.title()))?;
        app.caption();
        // client W x H DIPs, centered on the monitor under the mouse
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST), &mut mi);
        let s = app.scale();
        let mut r = RECT { left: 0, top: 0, right: (W * s).round() as i32, bottom: (H * s).round() as i32 };
        AdjustWindowRectExForDpi(&mut r, style, false, WINDOW_EX_STYLE::default(), GetDpiForWindow(hwnd))?;
        let (ww, wh, work) = (r.right - r.left, r.bottom - r.top, mi.rcWork);
        let (x, y) = (work.left + (work.right - work.left - ww) / 2, work.top + (work.bottom - work.top - wh) / 2);
        SetWindowPos(hwnd, None, x, y.max(work.top), ww, wh, SWP_NOZORDER | SWP_NOACTIVATE)?;
        APP.with(|a| *a.borrow_mut() = Some(app));
        // SW_SHOWDEFAULT: whoever started us decides (tests start it without focus)
        let _ = ShowWindow(hwnd, SW_SHOWDEFAULT);
        let mut m = MSG::default();
        while GetMessageW(&mut m, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
    }
    Ok(())
}
