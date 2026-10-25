//! regsvr32 registration (needs admin): the COM class, the input profile under
//! Chinese (Simplified), and the capabilities TSF checks before loading us.

use crate::{CLSID, DESC, PROFILE, dll};
use windows::{
    Win32::{
        Foundation::ERROR_SUCCESS,
        System::{Com::*, LibraryLoader::GetModuleFileNameW, Registry::*},
        UI::{Input::KeyboardAndMouse::HKL, TextServices::*},
    },
    core::*,
};

const LANG: u16 = 0x0804;

const CATEGORIES: [GUID; 7] = [
    GUID_TFCAT_TIP_KEYBOARD,
    // the composition's underline
    GUID_TFCAT_DISPLAYATTRIBUTEPROVIDER,
    // we write 中/英 and full width to the conversion mode compartment
    GUID_TFCAT_TIPCAP_INPUTMODECOMPARTMENT,
    // games and other apps that draw the candidate list themselves
    GUID_TFCAT_TIPCAP_UIELEMENTENABLED,
    // Store apps and the Start menu search box
    GUID_TFCAT_TIPCAP_IMMERSIVESUPPORT,
    // the 中/英 button in the taskbar
    GUID_TFCAT_TIPCAP_SYSTRAYSUPPORT,
    GUID_TFCAT_TIPCAP_COMLESS,
];

fn class_key() -> HSTRING {
    format!("Software\\Classes\\CLSID\\{{{CLSID:?}}}").into()
}

fn set(key: &str, name: Option<&str>, value: &str) -> Result<()> {
    let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let name = name.map(HSTRING::from);
    let r = unsafe {
        RegSetKeyValueW(
            HKEY_LOCAL_MACHINE,
            &HSTRING::from(key),
            name.as_ref().map_or(PCWSTR::null(), |n| PCWSTR(n.as_ptr())),
            REG_SZ.0,
            Some(data.as_ptr() as _),
            (data.len() * 2) as u32,
        )
    };
    if r == ERROR_SUCCESS { Ok(()) } else { Err(r.into()) }
}

pub fn register() -> Result<()> {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(dll()), &mut buf) } as usize;
    let path = String::from_utf16_lossy(&buf[..n]);
    let key = class_key().to_string();
    set(&key, None, DESC)?;
    set(&format!("{key}\\InprocServer32"), None, &path)?;
    set(&format!("{key}\\InprocServer32"), Some("ThreadingModel"), "Apartment")?;
    unsafe {
        let p: ITfInputProcessorProfileMgr = CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        let desc: Vec<u16> = DESC.encode_utf16().collect();
        p.RegisterProfile(&CLSID, LANG, &PROFILE, &desc, &buf[..n], 0, HKL::default(), 0, true, 0)?;
        let c: ITfCategoryMgr = CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for g in CATEGORIES {
            c.RegisterCategory(&CLSID, &g, &CLSID)?;
        }
    }
    Ok(())
}

pub fn unregister() -> Result<()> {
    unsafe {
        let c: ITfCategoryMgr = CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        for g in CATEGORIES {
            let _ = c.UnregisterCategory(&CLSID, &g, &CLSID);
        }
        let p: ITfInputProcessorProfileMgr = CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER)?;
        let _ = p.UnregisterProfile(&CLSID, LANG, &PROFILE, 0);
        // the calls above leave their keys behind, empty
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, &HSTRING::from(format!("Software\\Microsoft\\CTF\\TIP\\{{{CLSID:?}}}")));
        let _ = RegDeleteTreeW(HKEY_LOCAL_MACHINE, &class_key());
    }
    Ok(())
}
