//! End-to-end check of the installed IME: opens a RichEdit window, switches only
//! this process to Brushkey, types a script with SendInput and prints the text.
//!
//! `cargo run -p ime --example host -- [tsf|imm][+uiless][+password] "<script>"`
//! `uiless` makes the host draw candidates itself like a game: it prints them.
//! `password` marks the edit box as a password field (InputScope).
//! The script is typed as on a US keyboard; `{name}` is a special key: shift
//! (Shift alone), ctrl (Ctrl alone), enter, esc, bs, space, pgdn, tab, left, s+space, c+s+f,
//! shot (prints the visible IME windows; with SHOTS=<dir> also saves a BMP of
//! the screen around them), tray (with SHOTS: the taskbar corner), wait,
//! rclick (a right click on the IME's language bar button).
//! Keys go to the foreground window, so the run stops if this window loses focus.
//! Windows switches input methods for the whole desktop by default, so the one
//! active before is restored at the end; `host restore {clsid} {profile}`
//! activates a given one (zh-CN) when a run didn't get to do that.
//! `host configure` does what the keyboard's Options button in Windows settings
//! does: it asks the installed IME to show its settings.

use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Foundation::*,
        System::{
            Com::*,
            LibraryLoader::{GetModuleHandleW, LoadLibraryW},
            Threading::{AttachThreadInput, GetCurrentThreadId},
        },
        UI::{Input::KeyboardAndMouse::*, TextServices::*, WindowsAndMessaging::*},
    },
    core::*,
};

const CLSID: GUID = GUID::from_u128(0x3a51b2bb_2252_4927_8495_f19087e9e0d7);
const PROFILE: GUID = GUID::from_u128(0x4f552bca_fa84_4dfa_a856_bbffd6a440ed);

windows_core::link!("msctf.dll" "system" fn SetInputScope(hwnd: HWND, scope: InputScope) -> HRESULT);

fn pump(ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        unsafe {
            let mut m = MSG::default();
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn send(vk: u16, up: bool) {
    let ki = KEYBDINPUT {
        wVk: VIRTUAL_KEY(vk),
        wScan: unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) } as u16,
        dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
        ..Default::default()
    };
    let input = INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki } };
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) };
    pump(15);
}

/// Presses `vk` with the modifiers held.
fn press(mods: &[u16], vk: u16) {
    mods.iter().for_each(|&m| send(m, false));
    send(vk, false);
    send(vk, true);
    mods.iter().rev().for_each(|&m| send(m, true));
    pump(30);
}

/// The key and Shift state that type `c` on the US layout.
fn vk(c: char) -> (u16, bool) {
    let rows: [(u16, &str, &str); 3] = [(0x30, "0123456789", ")!@#$%^&*("), (0xBA, ";=,-./`", ":+<_>?~"), (0xDB, "[\\]'", "{|}\"")];
    match c {
        'a'..='z' => (c as u16 - 32, false),
        'A'..='Z' => (c as u16, true),
        ' ' => (0x20, false),
        _ => rows
            .iter()
            .find_map(|(base, a, b)| {
                a.chars().position(|x| x == c).map(|i| (base + i as u16, false)).or_else(|| b.chars().position(|x| x == c).map(|i| (base + i as u16, true)))
            })
            .unwrap_or_else(|| panic!("no key for {c:?}")),
    }
}

extern "system" fn wndproc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, m, w, l) }
}

extern "system" fn shown(h: HWND, l: LPARAM) -> BOOL {
    unsafe {
        let mut name = [0u16; 64];
        let n = GetClassNameW(h, &mut name) as usize;
        if IsWindowVisible(h).as_bool() && String::from_utf16_lossy(&name[..n]) == "BrushkeyCandidates" {
            let mut r = RECT::default();
            let _ = GetWindowRect(h, &mut r);
            println!("  window {}x{} at ({}, {})", r.right - r.left, r.bottom - r.top, r.left, r.top);
            let u = &mut *(l.0 as *mut RECT);
            *u = RECT { left: u.left.min(r.left), top: u.top.min(r.top), right: u.right.max(r.right), bottom: u.bottom.max(r.bottom) };
        }
    }
    TRUE
}

/// Saves the screen inside `r` as a 32-bit BMP.
fn save(r: RECT, path: &str) {
    use windows::Win32::Graphics::Gdi::*;
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bmp.into());
        let _ = BitBlt(dc, 0, 0, w, h, Some(screen), r.left, r.top, SRCCOPY);
        SelectObject(dc, old);
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER { biSize: 40, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, ..Default::default() },
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr() as _), &mut bi, DIB_RGB_COLORS);
        let mut f = b"BM".to_vec();
        f.extend((54 + px.len() as u32).to_le_bytes());
        f.extend([0u8; 4]);
        f.extend(54u32.to_le_bytes());
        f.extend(std::slice::from_raw_parts(&bi.bmiHeader as *const _ as *const u8, 40));
        f.extend(px);
        std::fs::write(path, f).unwrap();
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
    }
}

/// Takes the candidate list like a game that draws its own, printing each update.
#[windows_core::implement(ITfUIElementSink)]
struct Sink(ITfUIElementMgr);

impl ITfUIElementSink_Impl for Sink_Impl {
    fn BeginUIElement(&self, _id: u32, show: *mut BOOL) -> Result<()> {
        unsafe { *show = FALSE };
        Ok(())
    }

    fn UpdateUIElement(&self, id: u32) -> Result<()> {
        unsafe {
            let e: ITfCandidateListUIElement = self.0.GetUIElement(id)?.cast()?;
            let (n, sel, page) = (e.GetCount()?, e.GetSelection()?, e.GetCurrentPage()?);
            let cands: Vec<String> = (0..n).map(|i| e.GetString(i).map(|b| b.to_string()).unwrap_or_default()).collect();
            println!("  ui-less: page {page}, selected {sel}, {n} so far: {}", cands.join(" "));
        }
        Ok(())
    }

    fn EndUIElement(&self, _id: u32) -> Result<()> {
        println!("  ui-less: end");
        Ok(())
    }
}

fn text(edit: HWND) -> String {
    unsafe {
        let mut buf = vec![0u16; GetWindowTextLengthW(edit) as usize + 1];
        let n = GetWindowTextW(edit, &mut buf) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (tsf, script) = (args.first().is_some_and(|a| a.starts_with("tsf")), args.get(1).cloned().unwrap_or_default());
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let mgr: ITfInputProcessorProfileMgr = CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        if args.first().is_some_and(|a| a == "restore") {
            let g = |i: usize| GUID::try_from(args[i].trim_matches(|c| c == '{' || c == '}')).unwrap();
            return mgr.ActivateProfile(TF_PROFILETYPE_INPUTPROCESSOR, 0x0804, &g(1), &g(2), HKL::default(), TF_IPPMF_FORSESSION);
        }
        if args.first().is_some_and(|a| a == "configure") {
            let f: ITfFnConfigure = CoCreateInstance(&CLSID, None, CLSCTX_INPROC_SERVER)?;
            return f.Show(HWND::default(), 0x0804, &PROFILE);
        }
        let mut prev = TF_INPUTPROCESSORPROFILE::default();
        mgr.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut prev)?;
        println!("before: {:?}", prev.clsid);
        LoadLibraryW(w!("msftedit.dll"))?;
        let inst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: w!("BrushkeyHost"), ..Default::default() };
        RegisterClassW(&wc);
        let main = CreateWindowExW(WS_EX_TOPMOST, w!("BrushkeyHost"), w!("Brushkey host"), WS_OVERLAPPEDWINDOW | WS_VISIBLE, 200, 200, 700, 300, None, None, Some(inst.into()), None)?;
        let edit = CreateWindowExW(WINDOW_EX_STYLE(0), w!("RICHEDIT50W"), w!(""), WS_CHILD | WS_VISIBLE | WINDOW_STYLE(4), 10, 10, 660, 240, Some(main), None, Some(inst.into()), None)?;
        if tsf {
            // EM_SETEDITSTYLE, SES_USECTF: RichEdit talks TSF directly instead of through IMM32
            SendMessageW(edit, 0x4CC, Some(WPARAM(0x10000)), Some(LPARAM(0x10000)));
        }
        if args.first().is_some_and(|a| a.contains("password")) {
            SetInputScope(edit, IS_PASSWORD).ok()?;
        }
        let fg = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let _ = AttachThreadInput(GetCurrentThreadId(), fg, true);
        let _ = SetForegroundWindow(main);
        let _ = AttachThreadInput(GetCurrentThreadId(), fg, false);
        let _ = SetFocus(Some(edit));
        pump(300);
        mgr.ActivateProfile(TF_PROFILETYPE_INPUTPROCESSOR, 0x0804, &CLSID, &PROFILE, HKL::default(), TF_IPPMF_FORPROCESS | TF_IPPMF_DONTCARECURRENTINPUTLANGUAGE)?;
        pump(300);
        if args.first().is_some_and(|a| a.contains("uiless")) {
            let tm: ITfThreadMgr = CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
            let sink: ITfUIElementSink = Sink(tm.cast()?).into();
            tm.cast::<ITfSource>()?.AdviseSink(&ITfUIElementSink::IID, &sink)?;
        }
        let mut active = TF_INPUTPROCESSORPROFILE::default();
        mgr.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut active)?;
        println!("active: {}", if active.clsid == CLSID { "Brushkey" } else { "another input method" });
        let mut rest = script.as_str();
        let mut shots = 0;
        while let Some(c) = rest.chars().next() {
            if GetForegroundWindow() != main {
                println!("stopped: the window lost focus");
                break;
            }
            if c == '{' {
                let end = rest.find('}').expect("unclosed {");
                match &rest[1..end] {
                    "shift" => press(&[], 0x10),
                    "ctrl" => press(&[], 0x11),
                    "enter" => press(&[], 0x0D),
                    "esc" => press(&[], 0x1B),
                    "bs" => press(&[], 0x08),
                    "space" => press(&[], 0x20),
                    "pgdn" => press(&[], 0x22),
                    "tab" => press(&[], 0x09),
                    "left" => press(&[], 0x25),
                    "s+space" => press(&[0x10], 0x20),
                    "c+s+f" => press(&[0x11, 0x10], 0x46),
                    "shot" => {
                        println!("text: {:?}", text(edit));
                        let mut r = RECT::default();
                        let _ = GetWindowRect(main, &mut r);
                        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(shown), LPARAM(&mut r as *mut RECT as isize));
                        if let Ok(dir) = std::env::var("SHOTS") {
                            shots += 1;
                            save(r, &format!("{dir}/shot{shots}.bmp"));
                        }
                    }
                    "wait" => pump(300),
                    "rclick" => {
                        let items: ITfLangBarItemMgr = CoCreateInstance(&CLSID_TF_LangBarItemMgr, None, CLSCTX_INPROC_SERVER)?;
                        let b: ITfLangBarItemButton = items.GetItem(&GUID_LBI_INPUTMODE)?.cast()?;
                        b.OnClick(TF_LBI_CLK_RIGHT, POINT::default(), &RECT::default())?;
                    }
                    "tray" if std::env::var("SHOTS").is_ok() => {
                        let (w, h) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
                        shots += 1;
                        save(RECT { left: w - 360, top: h - 56, right: w, bottom: h }, &format!("{}/shot{shots}.bmp", std::env::var("SHOTS").unwrap()));
                    }
                    k => panic!("unknown key {k}"),
                }
                rest = &rest[end + 1..];
            } else {
                let (k, shift) = vk(c);
                press(if shift { &[0x10] } else { &[] }, k);
                rest = &rest[c.len_utf8()..];
            }
        }
        pump(200);
        println!("text: {:?}", text(edit));
        let _ = DestroyWindow(main);
        mgr.ActivateProfile(prev.dwProfileType, prev.langid, &prev.clsid, &prev.guidProfile, prev.hkl, TF_IPPMF_FORSESSION)?;
    }
    Ok(())
}
