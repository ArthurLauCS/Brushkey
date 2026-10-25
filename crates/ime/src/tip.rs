//! The TSF text service: one per thread of an app that takes text input. It
//! feeds keys to `Ime`, shows the result as an inline composition, and hands
//! the candidate list to the app (games draw their own) or to our window.

use crate::{
    ATTR, ATTR_FOCUS, ELEMENT, NAME, beside_dll, dict, guard,
    ime::{Ime, Key, Mode, Out},
    langbar::Button,
    load_mode, sandboxed, save_mode,
    settings::Settings,
    ui, user_dir,
};
use engine::{Options, User};
use std::{
    cell::{Cell, RefCell},
    mem::ManuallyDrop,
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::ClientToScreen,
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
            Threading::{CreateProcessW, GetCurrentThreadId, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW},
            Variant::VARIANT,
        },
        UI::{
            Input::KeyboardAndMouse::{GetKeyState, VK_CAPITAL, VK_CONTROL, VK_MENU, VK_SHIFT},
            TextServices::*,
            WindowsAndMessaging::{GUITHREADINFO, GetGUIThreadInfo},
        },
    },
    core::*,
};

#[implement(ITfTextInputProcessorEx, ITfThreadMgrEventSink, ITfKeyEventSink, ITfCompositionSink, ITfThreadFocusSink, ITfCompartmentEventSink, ITfDisplayAttributeProvider, ITfFnConfigure)]
#[derive(Default)]
pub struct Tip {
    s: RefCell<State>,
}

#[derive(Default)]
struct State {
    tm: Option<ITfThreadMgr>,
    id: u32,
    sinks: Vec<(ITfSource, u32)>,
    /// None when the dictionary is missing: the service then lets every key through.
    ime: Option<Ime>,
    /// The mode last written to the compartments and the mode file.
    mode: Option<Mode>,
    /// The settings file as last read.
    settings: Settings,
    comp: Option<ITfComposition>,
    /// Context of the composition, or of the last key.
    ctx: Option<ITfContext>,
    /// Candidate list element while candidates are up, and its id.
    elem: Option<(ComObject<Element>, u32)>,
    button: Option<ComObject<Button>>,
    /// The atoms that tag composition text with `ATTRS`; 0 if TSF gave none.
    attr: [u32; 2],
}

/// Display attributes: composition text, the word being adjusted.
const ATTRS: [GUID; 2] = [ATTR, ATTR_FOCUS];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn compartment(mgr: &impl Interface, g: &GUID) -> Result<ITfCompartment> {
    unsafe { mgr.cast::<ITfCompartmentMgr>()?.GetCompartment(g) }
}

fn get(mgr: &impl Interface, g: &GUID) -> i32 {
    compartment(mgr, g).and_then(|c| unsafe { c.GetValue() }).ok().and_then(|v| i32::try_from(&v).ok()).unwrap_or(0)
}

fn down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

impl Tip_Impl {
    fn ime<T>(&self, f: impl FnOnce(&mut Ime) -> T) -> Option<T> {
        self.s.borrow_mut().ime.as_mut().map(f)
    }

    /// The key for a key event, or None when the context doesn't take input
    /// (the app disabled the IME there, or there is nothing to type into).
    fn key(&self, ctx: &Ref<ITfContext>, w: WPARAM) -> Option<Key> {
        let ctx = ctx.as_ref()?;
        if self.s.borrow().ime.is_none() || get(ctx, &GUID_COMPARTMENT_KEYBOARD_DISABLED) != 0 || get(ctx, &GUID_COMPARTMENT_EMPTYCONTEXT) != 0 {
            return None;
        }
        let caps = unsafe { GetKeyState(VK_CAPITAL.0 as i32) & 1 != 0 };
        Some(Key { vk: w.0 as u16, shift: down(VK_SHIFT.0), ctrl: down(VK_CONTROL.0), alt: down(VK_MENU.0), caps })
    }

    /// Writes the mode to the compartments (apps and the system read 中/英 there)
    /// and the language bar; `save` also shares it with the other apps.
    fn sync(&self, save: bool) -> Result<()> {
        let (tm, id, m, button) = {
            let mut s = self.s.borrow_mut();
            let Some(m) = s.ime.as_ref().map(|i| i.mode) else { return Ok(()) };
            s.mode = Some(m);
            (s.tm.clone(), s.id, m, s.button.clone())
        };
        if save {
            save_mode(m);
        }
        if let Some(b) = button {
            b.update(m);
        }
        let Some(tm) = tm else { return Ok(()) };
        let conv = if m.chinese { TF_CONVERSIONMODE_NATIVE | TF_CONVERSIONMODE_SYMBOL } else { 0 } | if m.full { TF_CONVERSIONMODE_FULLSHAPE } else { 0 };
        unsafe {
            compartment(&tm, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)?.SetValue(id, &VARIANT::from(m.chinese as i32))?;
            compartment(&tm, &GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION)?.SetValue(id, &VARIANT::from(conv as i32))
        }
    }

    /// Applies what a key did: shares a changed mode, then updates the text.
    fn update(&self, ctx: &ITfContext, out: Out) -> Result<()> {
        let (changed, preedit, focus, id) = {
            let mut s = self.s.borrow_mut();
            s.ctx = Some(ctx.clone());
            let ime = s.ime.as_ref().unwrap();
            let preedit = if ime.engine.is_composing() { ime.engine.preedit() } else { String::new() };
            let focus = ime.engine.focus().map(|f| [f.start, f.end].map(|b| preedit[..b].encode_utf16().count() as i32));
            (s.mode != Some(ime.mode), preedit, focus, s.id)
        };
        if changed {
            self.sync(true)?;
        }
        let es: ITfEditSession = Session { tip: self.to_object(), ctx: ctx.clone(), commit: out.commit, preedit, focus, hint: out.hint }.into();
        unsafe { ctx.RequestEditSession(id, &es, TF_ES_ASYNCDONTCARE | TF_ES_READWRITE).map(|_| ()) }
    }

    /// Drops the composition and its text (focus moved away, or the app switched to English).
    fn cancel(&self) {
        let ctx = {
            let mut s = self.s.borrow_mut();
            if let Some(m) = &mut s.ime {
                m.engine.clear();
            }
            if s.comp.is_none() { None } else { s.ctx.clone() }
        };
        if let Some(ctx) = ctx {
            let _ = self.update(&ctx, Out::default());
        }
        ui::hide_hint();
    }

    /// Language bar click: toggles 中/英.
    pub fn click(&self) {
        let (out, ctx) = {
            let mut s = self.s.borrow_mut();
            let Some(ime) = &mut s.ime else { return };
            (ime.toggle(), s.ctx.clone())
        };
        match ctx {
            Some(ctx) => drop(self.update(&ctx, out)),
            None => drop(self.sync(true)),
        }
    }

    /// Runs inside an edit session: commits text, updates the composition, then
    /// shows the candidates and the mode hint at the text (adjusting: at the
    /// focused word, UTF-16 offsets `focus` in the preedit).
    fn edit(&self, ec: u32, ctx: &ITfContext, commit: &str, preedit: &str, focus: Option<[i32; 2]>, hint: Option<&'static str>) -> Result<()> {
        let (mut comp, attr) = {
            let s = self.s.borrow();
            (s.comp.clone(), s.attr)
        };
        // The underline is cosmetic: an app without the property still gets the text.
        let underline = |r: &ITfRange, atom: u32| unsafe {
            if let Ok(p) = ctx.GetProperty(&GUID_PROP_ATTRIBUTE) {
                let _ = if atom != 0 { p.SetValue(ec, r, &VARIANT::from(atom as i32)) } else { p.Clear(ec, r) };
            }
        };
        let mut at = None;
        unsafe {
            if !commit.is_empty() {
                let r = match comp.take() {
                    Some(c) => {
                        let r = c.GetRange()?;
                        r.SetText(ec, 0, &wide(commit))?;
                        underline(&r, 0);
                        c.EndComposition(ec)?;
                        r
                    }
                    None => ctx.cast::<ITfInsertAtSelection>()?.InsertTextAtSelection(ec, INSERT_TEXT_AT_SELECTION_FLAGS(0), &wide(commit))?,
                };
                caret(ec, ctx, &r, TF_ANCHOR_END)?;
            }
            if !preedit.is_empty() {
                let c = match comp.take() {
                    Some(c) => c,
                    None => {
                        let r = ctx.cast::<ITfInsertAtSelection>()?.InsertTextAtSelection(ec, TF_IAS_QUERYONLY, &[])?;
                        // Picks are made only while composing, so this is in place before the first one.
                        let private = password(ec, ctx, &r);
                        self.ime(|m| {
                            if m.engine.options().private != private {
                                m.engine.set_options(Options { private, ..m.engine.options() });
                            }
                        });
                        ctx.cast::<ITfContextComposition>()?.StartComposition(ec, &r, &self.to_interface::<ITfCompositionSink>())?
                    }
                };
                let r = c.GetRange()?;
                r.SetText(ec, 0, &wide(preedit))?;
                underline(&r, attr[0]);
                // The caret goes to the end, or to the focused word with its own underline.
                at = Some(match focus {
                    Some([a, b]) => {
                        let f = r.Clone()?;
                        let mut n = 0;
                        f.Collapse(ec, TF_ANCHOR_START)?;
                        f.ShiftEnd(ec, b, &mut n, std::ptr::null())?;
                        f.ShiftStart(ec, a, &mut n, std::ptr::null())?;
                        underline(&f, attr[1]);
                        caret(ec, ctx, &f, TF_ANCHOR_START)?;
                        f
                    }
                    None => {
                        caret(ec, ctx, &r, TF_ANCHOR_END)?;
                        r
                    }
                });
                comp = Some(c);
            } else if let Some(c) = comp.take() {
                c.GetRange()?.SetText(ec, 0, &[])?;
                c.EndComposition(ec)?;
            }
        }
        let rect = text_rect(ec, ctx, at.as_ref());
        self.s.borrow_mut().comp = comp;
        self.show(ctx, rect, hint)
    }

    /// Shows or hides the candidates: as a UI element for apps that draw them,
    /// in our window otherwise.
    fn show(&self, ctx: &ITfContext, rect: Option<RECT>, hint: Option<&'static str>) -> Result<()> {
        let (tm, elem, data) = {
            let s = &mut *self.s.borrow_mut();
            let data = match (&s.comp, &mut s.ime) {
                (Some(_), Some(ime)) => {
                    let page = ime.page_index();
                    let cands: Vec<String> = ime.cands().iter().map(|c| c.text.clone()).collect();
                    Some((cands, page, ime.per_page, ime.engine.preedit(), ime.mode.trad))
                }
                _ => None,
            };
            let elem = if data.is_some() { s.elem.clone() } else { s.elem.take() };
            (s.tm.clone(), elem, data)
        };
        let mgr = tm.as_ref().and_then(|tm| tm.cast::<ITfUIElementMgr>().ok());
        match (data, mgr) {
            (Some((cands, page, per, preedit, trad)), Some(mgr)) => unsafe {
                let (e, id) = match elem {
                    Some(e) => e,
                    None => {
                        // `per` set now: the app may read the pages inside BeginUIElement
                        let e = Element { d: RefCell::new(ElementData { per, ..Default::default() }) }.into_object();
                        let (mut show, mut id) = (TRUE, 0);
                        mgr.BeginUIElement(&e.to_interface::<ITfCandidateListUIElement>(), &mut show, &mut id)?;
                        e.d.borrow_mut().shown = show.as_bool();
                        self.s.borrow_mut().elem = Some((e.clone(), id));
                        (e, id)
                    }
                };
                let shown = {
                    let mut d = e.d.borrow_mut();
                    d.dim = ctx.GetDocumentMgr().ok();
                    d.page = page;
                    d.per = per;
                    d.cands = cands;
                    d.shown
                };
                mgr.UpdateUIElement(id)?;
                if shown {
                    let d = e.d.borrow();
                    ui::cands(rect, &preedit, &d.cands[(page * per).min(d.cands.len())..], trad, &self.s.borrow().settings);
                }
            },
            (_, mgr) => {
                if let (Some((_, id)), Some(mgr)) = (elem, mgr) {
                    unsafe { mgr.EndUIElement(id)? };
                }
                ui::hide();
            }
        }
        if let Some(h) = hint {
            ui::hint(rect, h);
        }
        Ok(())
    }
}

/// Moves the caret to the start or end of `r`.
unsafe fn caret(ec: u32, ctx: &ITfContext, r: &ITfRange, to: TfAnchor) -> Result<()> {
    unsafe {
        let r = r.Clone()?;
        r.Collapse(ec, to)?;
        let mut sel = [TF_SELECTION { range: ManuallyDrop::new(Some(r)), style: TF_SELECTIONSTYLE { ase: TF_AE_NONE, fInterimChar: FALSE } }];
        let res = ctx.SetSelection(ec, &sel);
        ManuallyDrop::drop(&mut sel[0].range);
        res
    }
}

/// Whether the app marked the text at `r` as a password.
fn password(ec: u32, ctx: &ITfContext, r: &ITfRange) -> bool {
    let scopes = || unsafe {
        let v = ctx.GetAppProperty(&GUID_PROP_INPUTSCOPE)?.GetValue(ec, r)?;
        let s: ITfInputScope = IUnknown::try_from(&v)?.cast()?;
        let (mut p, mut n) = (std::ptr::null_mut(), 0);
        s.GetInputScopes(&mut p, &mut n)?;
        let found = !p.is_null() && std::slice::from_raw_parts(p, n as usize).contains(&IS_PASSWORD);
        CoTaskMemFree(Some(p as _));
        Ok::<_, Error>(found)
    };
    scopes().unwrap_or(false)
}

/// Screen rectangle of the text `at`, or of the caret when not composing.
fn text_rect(ec: u32, ctx: &ITfContext, at: Option<&ITfRange>) -> Option<RECT> {
    let from_tsf = || unsafe {
        let range = match at {
            Some(r) => r.clone(),
            None => {
                let mut sel = [TF_SELECTION::default()];
                let mut n = 0;
                ctx.GetSelection(ec, TF_DEFAULT_SELECTION, &mut sel, &mut n)?;
                ManuallyDrop::take(&mut sel[0].range).ok_or(Error::from(E_FAIL))?
            }
        };
        let (mut rc, mut clipped) = (RECT::default(), FALSE);
        ctx.GetActiveView()?.GetTextExt(ec, &range, &mut rc, &mut clipped)?;
        Ok::<_, Error>(rc)
    };
    // Some apps report nothing; the system caret is the next best guess.
    from_tsf().ok().filter(|r| r.bottom > r.top || r.left != 0).or_else(|| unsafe {
        let mut gi = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        GetGUIThreadInfo(GetCurrentThreadId(), &mut gi).ok()?;
        if gi.hwndCaret.is_invalid() {
            return None;
        }
        let mut p = [POINT { x: gi.rcCaret.left, y: gi.rcCaret.top }, POINT { x: gi.rcCaret.left, y: gi.rcCaret.bottom }];
        for p in &mut p {
            let _ = ClientToScreen(gi.hwndCaret, p);
        }
        Some(RECT { left: p[0].x, top: p[0].y, right: p[0].x + 1, bottom: p[1].y })
    })
}

#[implement(ITfEditSession)]
struct Session {
    tip: ComObject<Tip>,
    ctx: ITfContext,
    commit: String,
    preedit: String,
    focus: Option<[i32; 2]>,
    hint: Option<&'static str>,
}

impl ITfEditSession_Impl for Session_Impl {
    fn DoEditSession(&self, ec: u32) -> Result<()> {
        guard(|| self.tip.edit(ec, &self.ctx, &self.commit, &self.preedit, self.focus, self.hint))
    }
}

impl ITfTextInputProcessor_Impl for Tip_Impl {
    fn Activate(&self, tm: Ref<ITfThreadMgr>, id: u32) -> Result<()> {
        self.ActivateEx(tm, id, 0)
    }

    fn Deactivate(&self) -> Result<()> {
        guard(|| unsafe {
            let s = std::mem::take(&mut *self.s.borrow_mut());
            if let Some(tm) = &s.tm {
                for (src, cookie) in s.sinks {
                    let _ = src.UnadviseSink(cookie);
                }
                if let Ok(k) = tm.cast::<ITfKeystrokeMgr>() {
                    let _ = k.UnadviseKeyEventSink(s.id);
                }
                if let (Ok(m), Some(b)) = (tm.cast::<ITfLangBarItemMgr>(), &s.button) {
                    let _ = m.RemoveItem(&b.to_interface::<ITfLangBarItemButton>());
                }
                if let (Ok(m), Some((_, id))) = (tm.cast::<ITfUIElementMgr>(), s.elem) {
                    let _ = m.EndUIElement(id);
                }
            }
            ui::destroy();
            Ok(())
        })
    }
}

impl ITfTextInputProcessorEx_Impl for Tip_Impl {
    fn ActivateEx(&self, tm: Ref<ITfThreadMgr>, id: u32, _flags: u32) -> Result<()> {
        guard(|| unsafe {
            let tm = tm.ok()?.clone();
            let Some(dict) = dict() else { return Ok(()) };
            let (mode, settings) = (load_mode(), Settings::load());
            let user = User::open(user_dir().map(|d| d.join("user.log")), !sandboxed());
            let button = Button::new(self.to_object(), mode).into_object();
            let cat = CoCreateInstance::<_, ITfCategoryMgr>(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER).ok();
            let attr = ATTRS.map(|g| cat.as_ref().and_then(|c| c.RegisterGUID(&g).ok()).unwrap_or(0));
            let ime = Some(Ime::new(dict, user, mode, &settings));
            *self.s.borrow_mut() = State { tm: Some(tm.clone()), id, ime, settings, button: Some(button.clone()), attr, ..Default::default() };
            let src: ITfSource = tm.cast()?;
            let open: ITfSource = compartment(&tm, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE)?.cast()?;
            let sinks = vec![
                (src.clone(), src.AdviseSink(&ITfThreadMgrEventSink::IID, &self.to_interface::<ITfThreadMgrEventSink>())?),
                (src.clone(), src.AdviseSink(&ITfThreadFocusSink::IID, &self.to_interface::<ITfThreadFocusSink>())?),
                (open.clone(), open.AdviseSink(&ITfCompartmentEventSink::IID, &self.to_interface::<ITfCompartmentEventSink>())?),
            ];
            self.s.borrow_mut().sinks = sinks;
            tm.cast::<ITfKeystrokeMgr>()?.AdviseKeyEventSink(id, &self.to_interface::<ITfKeyEventSink>(), true)?;
            tm.cast::<ITfLangBarItemMgr>()?.AddItem(&button.to_interface::<ITfLangBarItemButton>())?;
            self.sync(false)
        })
    }
}

impl ITfKeyEventSink_Impl for Tip_Impl {
    fn OnSetFocus(&self, _foreground: BOOL) -> Result<()> {
        Ok(())
    }

    fn OnTestKeyDown(&self, ctx: Ref<ITfContext>, w: WPARAM, _l: LPARAM) -> Result<BOOL> {
        guard(|| Ok(self.key(&ctx, w).and_then(|k| self.ime(|m| m.test(k))).unwrap_or(false).into()))
    }

    fn OnKeyDown(&self, ctx: Ref<ITfContext>, w: WPARAM, _l: LPARAM) -> Result<BOOL> {
        guard(|| {
            let Some(out) = self.key(&ctx, w).and_then(|k| self.ime(|m| m.down(k)).flatten()) else { return Ok(FALSE) };
            self.update(ctx.ok()?, out)?;
            Ok(TRUE)
        })
    }

    fn OnTestKeyUp(&self, ctx: Ref<ITfContext>, w: WPARAM, l: LPARAM) -> Result<BOOL> {
        self.OnKeyUp(ctx, w, l)
    }

    /// Never eats a key-up (see `Ime::up`); a Shift toggle happens on the first of the two calls.
    fn OnKeyUp(&self, ctx: Ref<ITfContext>, w: WPARAM, _l: LPARAM) -> Result<BOOL> {
        guard(|| {
            if self.key(&ctx, w).is_some() {
                if let Some(out) = self.ime(|m| m.up(w.0 as u16)).flatten() {
                    self.update(ctx.ok()?, out)?;
                }
            }
            Ok(FALSE)
        })
    }

    fn OnPreservedKey(&self, _ctx: Ref<ITfContext>, _g: *const GUID) -> Result<BOOL> {
        Ok(FALSE)
    }
}

impl ITfCompositionSink_Impl for Tip_Impl {
    /// The app ended the composition (a click elsewhere, focus moved): its pinyin is dropped, not committed.
    fn OnCompositionTerminated(&self, ec: u32, comp: Ref<ITfComposition>) -> Result<()> {
        guard(|| unsafe {
            if let Some(c) = comp.as_ref() {
                let _ = c.GetRange().and_then(|r| r.SetText(ec, 0, &[]));
            }
            let (tm, elem) = {
                let mut s = self.s.borrow_mut();
                s.comp = None;
                if let Some(m) = &mut s.ime {
                    m.engine.clear();
                }
                (s.tm.clone(), s.elem.take())
            };
            if let (Some(tm), Some((_, id))) = (tm, elem) {
                let _ = tm.cast::<ITfUIElementMgr>()?.EndUIElement(id);
            }
            ui::hide();
            Ok(())
        })
    }
}

impl ITfThreadMgrEventSink_Impl for Tip_Impl {
    fn OnInitDocumentMgr(&self, _d: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnUninitDocumentMgr(&self, _d: Ref<ITfDocumentMgr>) -> Result<()> {
        Ok(())
    }

    fn OnSetFocus(&self, _focus: Ref<ITfDocumentMgr>, _prev: Ref<ITfDocumentMgr>) -> Result<()> {
        guard(|| Ok(self.cancel()))
    }

    fn OnPushContext(&self, _c: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }

    fn OnPopContext(&self, _c: Ref<ITfContext>) -> Result<()> {
        Ok(())
    }
}

impl ITfThreadFocusSink_Impl for Tip_Impl {
    /// Picks up a mode switched in another app and changed settings.
    fn OnSetThreadFocus(&self) -> Result<()> {
        guard(|| {
            let st = Settings::load();
            if let State { ime: Some(ime), settings, .. } = &mut *self.s.borrow_mut() {
                if *settings != st {
                    ime.configure(&st);
                    *settings = st;
                }
            }
            let m = load_mode();
            if self.ime(|i| (i.mode != m).then(|| i.set_mode(m)).is_some()) == Some(true) {
                self.sync(false)?;
            }
            Ok(())
        })
    }

    fn OnKillThreadFocus(&self) -> Result<()> {
        guard(|| Ok(self.cancel()))
    }
}

impl ITfCompartmentEventSink_Impl for Tip_Impl {
    /// The app opened or closed the IME (games do around their chat box): follow
    /// it here without making it the mode of every app.
    fn OnChange(&self, g: *const GUID) -> Result<()> {
        guard(|| {
            let tm = self.s.borrow().tm.clone();
            let (Some(tm), true) = (tm, unsafe { *g } == GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) else { return Ok(()) };
            let open = get(&tm, &GUID_COMPARTMENT_KEYBOARD_OPENCLOSE) != 0;
            let (button, mode) = {
                let mut s = self.s.borrow_mut();
                let Some(i) = s.ime.as_mut().filter(|i| i.mode.chinese != open) else { return Ok(()) };
                i.set_mode(Mode { chinese: open, ..i.mode });
                let m = i.mode;
                s.mode = Some(m);
                (s.button.clone(), m)
            };
            if !open {
                self.cancel();
            }
            if let Some(b) = button {
                b.update(mode);
            }
            Ok(())
        })
    }
}

/// The candidate list as a UI element: apps that draw their own (games) read it
/// from here; the others leave it to us (`shown`).
#[implement(ITfCandidateListUIElement)]
#[derive(Default)]
pub struct Element {
    d: RefCell<ElementData>,
}

#[derive(Default)]
struct ElementData {
    /// Candidates up to the end of the current page.
    cands: Vec<String>,
    page: usize,
    /// Candidates per page.
    per: usize,
    shown: bool,
    dim: Option<ITfDocumentMgr>,
}

impl ITfUIElement_Impl for Element_Impl {
    fn GetDescription(&self) -> Result<BSTR> {
        Ok(NAME.into())
    }

    fn GetGUID(&self) -> Result<GUID> {
        Ok(ELEMENT)
    }

    fn Show(&self, show: BOOL) -> Result<()> {
        self.d.borrow_mut().shown = show.as_bool();
        if !show.as_bool() {
            ui::hide();
        }
        Ok(())
    }

    fn IsShown(&self) -> Result<BOOL> {
        Ok(self.d.borrow().shown.into())
    }
}

impl ITfCandidateListUIElement_Impl for Element_Impl {
    fn GetUpdatedFlags(&self) -> Result<u32> {
        Ok(TF_CLUIE_DOCUMENTMGR | TF_CLUIE_COUNT | TF_CLUIE_SELECTION | TF_CLUIE_STRING | TF_CLUIE_PAGEINDEX | TF_CLUIE_CURRENTPAGE)
    }

    fn GetDocumentMgr(&self) -> Result<ITfDocumentMgr> {
        self.d.borrow().dim.clone().ok_or(E_FAIL.into())
    }

    fn GetCount(&self) -> Result<u32> {
        Ok(self.d.borrow().cands.len() as u32)
    }

    fn GetSelection(&self) -> Result<u32> {
        let d = self.d.borrow();
        Ok((d.page * d.per) as u32)
    }

    fn GetString(&self, i: u32) -> Result<BSTR> {
        self.d.borrow().cands.get(i as usize).map(|s| BSTR::from(s.as_str())).ok_or(E_INVALIDARG.into())
    }

    fn GetPageIndex(&self, index: *mut u32, size: u32, count: *mut u32) -> Result<()> {
        let per = self.d.borrow().per;
        let pages = self.d.borrow().cands.len().div_ceil(per).max(1);
        unsafe {
            *count = pages as u32;
            if !index.is_null() {
                for i in 0..pages.min(size as usize) {
                    *index.add(i) = (i * per) as u32;
                }
            }
        }
        Ok(())
    }

    fn SetPageIndex(&self, _index: *const u32, _count: u32) -> Result<()> {
        Ok(())
    }

    fn GetCurrentPage(&self) -> Result<u32> {
        Ok(self.d.borrow().page as u32)
    }
}

impl ITfDisplayAttributeProvider_Impl for Tip_Impl {
    fn EnumDisplayAttributeInfo(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        Ok(AttrEnum::default().into())
    }

    fn GetDisplayAttributeInfo(&self, guid: *const GUID) -> Result<ITfDisplayAttributeInfo> {
        match ATTRS.iter().position(|g| *g == unsafe { *guid }) {
            Some(k) => Ok(Attr(k).into()),
            None => Err(E_INVALIDARG.into()),
        }
    }
}

impl ITfFunction_Impl for Tip_Impl {
    fn GetDisplayName(&self) -> Result<BSTR> {
        Ok(NAME.into())
    }
}

impl ITfFnConfigure_Impl for Tip_Impl {
    /// The keyboard's Options button in settings of Windows versions that offer
    /// it (Windows 11 doesn't for other vendors' input methods).
    fn Show(&self, _parent: HWND, _lang: u16, _profile: *const GUID) -> Result<()> {
        guard(open_settings)
    }
}

/// Starts the settings app next to the DLL; also a right click on the language
/// bar button. Not from a sandboxed app: the settings app would start in the
/// same sandbox and could not save.
pub fn open_settings() -> Result<()> {
    if sandboxed() {
        return Ok(());
    }
    unsafe {
        let exe = HSTRING::from(beside_dll("brushkey-settings.exe").as_os_str());
        let si = STARTUPINFOW { cb: size_of::<STARTUPINFOW>() as u32, ..Default::default() };
        let mut pi = PROCESS_INFORMATION::default();
        CreateProcessW(&exe, None, None, None, false, PROCESS_CREATION_FLAGS(0), None, None, &si, &mut pi)?;
        let _ = (CloseHandle(pi.hProcess), CloseHandle(pi.hThread));
    }
    Ok(())
}

/// `ATTRS[k]`: composition text with a dotted underline in the text's color; the
/// word being adjusted with a solid one, marked as the clause being converted.
#[implement(ITfDisplayAttributeInfo)]
struct Attr(usize);

impl ITfDisplayAttributeInfo_Impl for Attr_Impl {
    fn GetGUID(&self) -> Result<GUID> {
        Ok(ATTRS[self.0])
    }

    fn GetDescription(&self) -> Result<BSTR> {
        Ok(NAME.into())
    }

    fn GetAttributeInfo(&self, pda: *mut TF_DISPLAYATTRIBUTE) -> Result<()> {
        let (style, attr) = [(TF_LS_DOT, TF_ATTR_INPUT), (TF_LS_SOLID, TF_ATTR_TARGET_CONVERTED)][self.0];
        unsafe { *pda = TF_DISPLAYATTRIBUTE { lsStyle: style, bAttr: attr, ..Default::default() } };
        Ok(())
    }

    fn SetAttributeInfo(&self, _pda: *const TF_DISPLAYATTRIBUTE) -> Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn Reset(&self) -> Result<()> {
        Ok(())
    }
}

/// Lists our attributes.
#[implement(IEnumTfDisplayAttributeInfo)]
#[derive(Default)]
struct AttrEnum {
    next: Cell<usize>,
}

impl IEnumTfDisplayAttributeInfo_Impl for AttrEnum_Impl {
    fn Clone(&self) -> Result<IEnumTfDisplayAttributeInfo> {
        Ok(AttrEnum { next: self.next.clone() }.into())
    }

    fn Next(&self, count: u32, info: *mut Option<ITfDisplayAttributeInfo>, fetched: *mut u32) -> Result<()> {
        let k = self.next.get();
        let res = self.Skip(count);
        let n = self.next.get() - k;
        unsafe {
            for i in 0..n {
                info.add(i).write(Some(Attr(k + i).into()));
            }
            if !fetched.is_null() {
                *fetched = n as u32;
            }
        }
        res
    }

    fn Reset(&self) -> Result<()> {
        self.next.set(0);
        Ok(())
    }

    fn Skip(&self, count: u32) -> Result<()> {
        let k = self.next.get();
        let n = (count as usize).min(ATTRS.len() - k);
        self.next.set(k + n);
        if n == count as usize { Ok(()) } else { Err(S_FALSE.into()) }
    }
}
