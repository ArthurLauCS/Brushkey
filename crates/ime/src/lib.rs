//! Brushkey: a pinyin text service (TSF) for Windows. This file holds the DLL
//! entry points and what all threads of a process share: the dictionary, the
//! user's directory and the global mode. The settings app links it as a library
//! for the user's directory, the settings file and the theme.

mod ime;
mod langbar;
mod reg;
pub mod settings;
mod tip;
mod ui;

use engine::Dict;
use ime::Mode;
pub use ui::light;
use std::{
    ffi::c_void,
    os::windows::{ffi::OsStringExt, io::AsRawHandle},
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicPtr, Ordering},
    },
};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        System::{
            Com::{IClassFactory, IClassFactory_Impl},
            LibraryLoader::GetModuleFileNameW,
            Memory::*,
            SystemServices::DLL_PROCESS_ATTACH,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    },
    core::*,
};

pub const CLSID: GUID = GUID::from_u128(0x3a51b2bb_2252_4927_8495_f19087e9e0d7);
pub const PROFILE: GUID = GUID::from_u128(0x4f552bca_fa84_4dfa_a856_bbffd6a440ed);
/// Identifies our candidate list to apps that draw it themselves.
pub const ELEMENT: GUID = GUID::from_u128(0xfd13002e_6001_47f6_a5dc_42abb0079638);
/// The look of the composition text (dotted underline); apps draw it.
pub const ATTR: GUID = GUID::from_u128(0x79940892_96fa_4304_8db9_931377fb4f5f);
pub const NAME: &str = "Brushkey";
pub const DESC: &str = "Brushkey 拼音";

static DLL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub fn dll() -> HMODULE {
    HMODULE(DLL.load(Ordering::Relaxed))
}

/// Runs a COM method body; a panic becomes an error instead of unwinding into
/// the host app (games included), which would crash it.
pub fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(E_UNEXPECTED.into()))
}

/// A file in the install directory, next to the DLL.
pub fn beside_dll(name: &str) -> PathBuf {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(Some(dll()), &mut buf) } as usize;
    PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])).with_file_name(name)
}

/// The dictionary next to the DLL, memory-mapped so all processes share its pages.
pub fn dict() -> Option<&'static Dict> {
    static D: OnceLock<Option<Dict>> = OnceLock::new();
    D.get_or_init(|| unsafe {
        let f = std::fs::File::open(beside_dll("dict.bin")).ok()?;
        let len = f.metadata().ok()?.len() as usize;
        let map = CreateFileMappingW(HANDLE(f.as_raw_handle()), None, PAGE_READONLY, 0, 0, None).ok()?;
        let view = MapViewOfFile(map, FILE_MAP_READ, 0, 0, 0);
        let _ = CloseHandle(map); // the view keeps the mapping alive
        if view.Value.is_null() {
            return None;
        }
        Dict::load(std::slice::from_raw_parts(view.Value as *const u8, len))
    })
    .as_ref()
}

/// Sandboxed (AppContainer) processes may read the user's directory but not write it.
pub fn sandboxed() -> bool {
    static S: OnceLock<bool> = OnceLock::new();
    *S.get_or_init(|| unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return true;
        }
        let (mut v, mut n) = (0u32, 0u32);
        let r = GetTokenInformation(token, TokenIsAppContainer, Some(&mut v as *mut u32 as _), 4, &mut n);
        let _ = CloseHandle(token);
        r.is_err() || v != 0
    })
}

/// `%APPDATA%\Brushkey`, created readable by sandboxed apps (they read memory and mode).
pub fn user_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join(NAME);
    if !sandboxed() && std::fs::create_dir(&dir).is_ok() {
        unsafe {
            // The directory inherits the user's access from %APPDATA%; this adds
            // read for all app containers, restricted ones included.
            let mut sd = PSECURITY_DESCRIPTOR::default();
            let sddl = w!("D:(A;OICI;GRGX;;;AC)(A;OICI;GRGX;;;S-1-15-2-2)");
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl, SDDL_REVISION_1, &mut sd, None).is_ok() {
                let (mut present, mut dacl, mut defaulted) = (FALSE, std::ptr::null_mut(), FALSE);
                if GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted).is_ok() {
                    let info = DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION;
                    let _ = SetNamedSecurityInfoW(&HSTRING::from(dir.as_os_str()), SE_FILE_OBJECT, info, None, None, Some(dacl), None);
                }
                let _ = LocalFree(Some(HLOCAL(sd.0)));
            }
        }
    }
    Some(dir)
}

/// The mode shared by all apps: 中/英, full width, 简/繁; one '0'/'1' each.
pub fn load_mode() -> Mode {
    let b = user_dir().and_then(|d| std::fs::read(d.join("mode")).ok()).unwrap_or_default();
    let on = |i: usize, def: bool| b.get(i).map_or(def, |&c| c == b'1');
    Mode { chinese: on(0, true), full: on(1, false), trad: on(2, false) }
}

pub fn save_mode(m: Mode) {
    if let Some(d) = user_dir().filter(|_| !sandboxed()) {
        let _ = std::fs::write(d.join("mode"), [m.chinese, m.full, m.trad].map(|b| b'0' + b as u8));
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<IUnknown>, iid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        guard(|| unsafe {
            *out = std::ptr::null_mut();
            if outer.is_some() {
                return Err(CLASS_E_NOAGGREGATION.into());
            }
            let unk: IUnknown = tip::Tip::default().into();
            unk.query(iid, out).ok()
        })
    }

    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[unsafe(no_mangle)]
extern "system" fn DllMain(h: HMODULE, reason: u32, _: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        DLL.store(h.0, Ordering::Relaxed);
    }
    TRUE
}

#[unsafe(no_mangle)]
extern "system" fn DllGetClassObject(clsid: *const GUID, iid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    unsafe {
        *out = std::ptr::null_mut();
        if *clsid != CLSID {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        IClassFactory::from(Factory).query(iid, out)
    }
}

/// Never unloads: the dictionary mapping and window class live for the process.
#[unsafe(no_mangle)]
extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}

#[unsafe(no_mangle)]
extern "system" fn DllRegisterServer() -> HRESULT {
    guard(reg::register).into()
}

#[unsafe(no_mangle)]
extern "system" fn DllUnregisterServer() -> HRESULT {
    guard(reg::unregister).into()
}
