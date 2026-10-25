//! The 简/繁/英 button the taskbar shows for the input method; a left click
//! toggles 中/英, a right click opens the settings app.

use crate::{
    CLSID, DESC, NAME,
    ime::Mode,
    tip::{Tip, open_settings},
    ui,
};
use std::cell::{Cell, RefCell};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::Ole::CONNECT_E_CANNOTCONNECT,
        UI::{TextServices::*, WindowsAndMessaging::*},
    },
    core::*,
};

#[implement(ITfLangBarItemButton, ITfSource)]
pub struct Button {
    tip: ComObject<Tip>,
    text: Cell<&'static str>,
    sink: RefCell<Option<ITfLangBarItemSink>>,
}

impl Button {
    pub fn new(tip: ComObject<Tip>, mode: Mode) -> Button {
        Button { tip, text: Cell::new(text(mode)), sink: RefCell::new(None) }
    }

    pub fn update(&self, mode: Mode) {
        self.text.set(text(mode));
        let sink = self.sink.borrow().clone();
        if let Some(s) = sink {
            let _ = unsafe { s.OnUpdate(TF_LBI_ICON | TF_LBI_TEXT) };
        }
    }

}

/// Chinese mode shows the script, since 简/繁 applies to every app.
fn text(m: Mode) -> &'static str {
    match (m.chinese, m.trad) {
        (false, _) => "英",
        (true, false) => "简",
        (true, true) => "繁",
    }
}

impl ITfLangBarItem_Impl for Button_Impl {
    fn GetInfo(&self, info: *mut TF_LANGBARITEMINFO) -> Result<()> {
        let mut desc = [0u16; 32];
        desc.iter_mut().zip(NAME.encode_utf16()).for_each(|(d, c)| *d = c);
        unsafe {
            *info = TF_LANGBARITEMINFO {
                clsidService: CLSID,
                guidItem: GUID_LBI_INPUTMODE,
                dwStyle: TF_LBI_STYLE_BTN_BUTTON | TF_LBI_STYLE_SHOWNINTRAY,
                ulSort: 0,
                szDescription: desc,
            }
        };
        Ok(())
    }

    fn GetStatus(&self) -> Result<u32> {
        Ok(0)
    }

    fn Show(&self, _show: BOOL) -> Result<()> {
        Ok(())
    }

    fn GetTooltipString(&self) -> Result<BSTR> {
        Ok(DESC.into())
    }
}

impl ITfLangBarItemButton_Impl for Button_Impl {
    fn OnClick(&self, click: TfLBIClick, _pt: &POINT, _area: *const RECT) -> Result<()> {
        crate::guard(|| match click {
            TF_LBI_CLK_LEFT => Ok(self.tip.click()),
            TF_LBI_CLK_RIGHT => open_settings(),
            _ => Ok(()),
        })
    }

    fn InitMenu(&self, _menu: Ref<ITfMenu>) -> Result<()> {
        Ok(())
    }

    fn OnMenuSelect(&self, _id: u32) -> Result<()> {
        Ok(())
    }

    fn GetIcon(&self) -> Result<HICON> {
        crate::guard(|| icon(self.text.get()))
    }

    fn GetText(&self) -> Result<BSTR> {
        Ok(self.text.get().into())
    }
}

impl ITfSource_Impl for Button_Impl {
    fn AdviseSink(&self, iid: *const GUID, sink: Ref<IUnknown>) -> Result<u32> {
        if unsafe { *iid } != ITfLangBarItemSink::IID {
            return Err(CONNECT_E_CANNOTCONNECT.into());
        }
        *self.sink.borrow_mut() = Some(sink.ok()?.cast()?);
        Ok(1)
    }

    fn UnadviseSink(&self, _cookie: u32) -> Result<()> {
        *self.sink.borrow_mut() = None;
        Ok(())
    }
}

/// A small icon with one character, in the taskbar's text color. The caller destroys it.
fn icon(text: &str) -> Result<HICON> {
    unsafe {
        let size = GetSystemMetrics(SM_CXSMICON);
        let dc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size,
                biHeight: -size,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let color = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let old = SelectObject(dc, color.into());
        let font = CreateFontW(-size * 15 / 16, 0, 0, 0, FW_BOLD.0 as i32, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, w!("Microsoft YaHei UI"));
        let old_font = SelectObject(dc, font.into());
        SetTextColor(dc, COLORREF(0xFFFFFF));
        SetBkMode(dc, TRANSPARENT);
        let mut rc = RECT { left: 0, top: 0, right: size, bottom: size };
        let mut t: Vec<u16> = text.encode_utf16().collect();
        DrawTextW(dc, &mut t, &mut rc, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        // White text on black: the brightness is the coverage, used as alpha.
        let fg = if ui::light("SystemUsesLightTheme") { 0x000000 } else { 0xFFFFFF };
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
        for p in px {
            *p = ((*p >> 8) & 0xFF) << 24 | fg;
        }
        SelectObject(dc, old_font);
        SelectObject(dc, old);
        let _ = DeleteObject(font.into());
        let mask = CreateBitmap(size, size, 1, 1, None);
        let icon = CreateIconIndirect(&ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color });
        let _ = DeleteObject(mask.into());
        let _ = DeleteObject(color.into());
        let _ = DeleteDC(dc);
        icon
    }
}
