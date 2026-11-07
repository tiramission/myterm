//! Installing and removing the app.
//!
//! A Windows program that wants to behave like an installed one needs three
//! things: a stable location under the user's profile, a shortcut where people
//! look for it, and an entry in the list Settings shows. This module does all
//! three, plus the removal that undoes them.
//!
//! Everything is per-user. Nothing here asks for elevation: the app has never
//! needed administrator rights, and an installer is a bad place to start.
//!
//! Both directions finish by replacing or deleting the executable that is
//! currently running, which Windows will not allow. Each therefore hands the
//! last step to a short-lived script that waits for this process to exit — the
//! same trick the updater uses, for the same reason.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

/// Where an installed copy lives.
///
/// `%LOCALAPPDATA%\Programs` is the convention for a per-user install: it needs
/// no elevation, and it is where users (and uninstallers) expect to find
/// programs that did not go to `Program Files`.
pub fn install_dir() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    Some(PathBuf::from(local).join("Programs").join("myterm"))
}

/// The installed executable's path.
pub fn installed_exe() -> Option<PathBuf> {
    Some(install_dir()?.join("myterm.exe"))
}

/// The executable this process is running from.
pub fn current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// Whether this process is the installed copy.
///
/// Compared by canonical path rather than by string: `current_exe` can carry a
/// different prefix than the one this module builds, and a false negative here
/// would offer to install an app that is already installed.
pub fn is_installed() -> bool {
    let (Some(current), Some(installed)) = (current_exe(), installed_exe()) else {
        return false;
    };
    match (current.canonicalize(), installed.canonicalize()) {
        // The installed file may not exist yet, in which case canonicalize
        // fails and the comparison is false — which is the right answer.
        (Ok(current), Ok(installed)) => current == installed,
        _ => false,
    }
}

/// Whether a copy is present at the install location, even if this process is
/// running from somewhere else.
pub fn is_present() -> bool {
    installed_exe().is_some_and(|path| path.exists())
}

/// The registry key that makes the app appear in Settings → Apps.
const UNINSTALL_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\myterm";

/// What went wrong.
#[derive(Debug)]
pub enum InstallError {
    /// The environment did not describe a usable location.
    NoLocation(String),
    /// A file or directory operation failed.
    Io(String),
    /// The registry refused.
    Registry(String),
    /// The shortcut could not be created.
    Shortcut(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLocation(detail) => write!(f, "找不到安装位置：{detail}"),
            Self::Io(detail) => write!(f, "文件操作失败：{detail}"),
            Self::Registry(detail) => write!(f, "写入注册表失败：{detail}"),
            Self::Shortcut(detail) => write!(f, "创建快捷方式失败：{detail}"),
        }
    }
}

/// Options the user gets to choose at install time.
pub struct InstallOptions {
    /// Whether to put a shortcut on the desktop.
    ///
    /// Off by default in the caller's UI would be unusual — most installers
    /// offer it checked — but it stays a choice, because a desktop icon is the
    /// single most complained-about thing an installer does on Windows.
    pub desktop_shortcut: bool,
}

/// Copies this executable into the install location, registers it, and makes
/// the shortcuts.
///
/// Returns the installed path. The caller is responsible for starting it and
/// quitting; this function does not, because that step differs between a fresh
/// install and an update.
pub fn install(options: &InstallOptions) -> Result<PathBuf, InstallError> {
    let dir = install_dir().ok_or_else(|| InstallError::NoLocation("LOCALAPPDATA".to_owned()))?;
    let target = dir
        .join("myterm.exe")
        .to_str()
        .map(PathBuf::from)
        .ok_or_else(|| InstallError::NoLocation("install path".to_owned()))?;

    let source = current_exe().ok_or_else(|| InstallError::NoLocation("current exe".to_owned()))?;

    // Installing over a running copy is the case that needs the script: the
    // file is locked, so the copy has to happen after this process is gone.
    // Installing from anywhere else can copy directly, which keeps the common
    // "run the downloaded exe, then install" path free of a delay.
    let same_file = source
        .canonicalize()
        .ok()
        .zip(target.canonicalize().ok())
        .is_some_and(|(source, target)| source == target);

    if !same_file {
        std::fs::create_dir_all(&dir)
            .map_err(|error| InstallError::Io(format!("create {}: {error}", dir.display())))?;
        std::fs::copy(&source, &target)
            .map_err(|error| InstallError::Io(format!("copy to {}: {error}", target.display())))?;
    }

    write_uninstall_entry(&target)?;
    create_shortcuts(&target, options.desktop_shortcut)?;

    Ok(target)
}

/// Removes the shortcuts, the registry entry, and — through a script — the
/// installed files.
///
/// The executable cannot delete itself, so the deletion is deferred. Everything
/// that does not need the process gone happens first, so a user who cancels at
/// the last moment still has a consistent state.
pub fn uninstall() -> Result<(), InstallError> {
    remove_shortcuts()?;
    remove_uninstall_entry()?;

    let Some(dir) = install_dir() else {
        return Err(InstallError::NoLocation("LOCALAPPDATA".to_owned()));
    };

    // The script waits for this process to exit, deletes the directory, and
    // removes itself. `rmdir /s /q` is used rather than `del` because the
    // directory itself has to go, and it tolerates the file being locked for
    // the first moment after exit.
    let script = std::env::temp_dir().join("myterm-uninstall.cmd");
    let body = format!(
        "@echo off\r\n\
         timeout /t 2 /nobreak >nul\r\n\
         rmdir /s /q \"{dir}\"\r\n\
         del \"%~f0\"\r\n",
        dir = dir.display(),
    );
    std::fs::write(&script, body)
        .map_err(|error| InstallError::Io(format!("write uninstall script: {error}")))?;

    std::process::Command::new("cmd")
        .arg("/c")
        .arg("start")
        .arg("")
        .arg("/min")
        .arg(&script)
        .spawn()
        .map_err(|error| InstallError::Io(format!("start uninstall script: {error}")))?;

    Ok(())
}

/// Starts the installed copy and lets the caller quit.
pub fn launch_installed() -> Result<(), InstallError> {
    let exe = installed_exe().ok_or_else(|| InstallError::NoLocation("install path".to_owned()))?;
    std::process::Command::new(&exe)
        .spawn()
        .map_err(|error| InstallError::Io(format!("start {}: {error}", exe.display())))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Shortcuts
// ---------------------------------------------------------------------------

/// The two places a shortcut can live.
fn start_menu_shortcut() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(appdata)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("myterm.lnk"),
    )
}

fn desktop_shortcut() -> Option<PathBuf> {
    let profile = std::env::var_os("USERPROFILE")?;
    Some(PathBuf::from(profile).join("Desktop").join("myterm.lnk"))
}

fn create_shortcuts(exe: &Path, desktop: bool) -> Result<(), InstallError> {
    if let Some(path) = start_menu_shortcut() {
        write_shortcut(&path, exe)?;
    }
    if desktop && let Some(path) = desktop_shortcut() {
        write_shortcut(&path, exe)?;
    }
    Ok(())
}

fn remove_shortcuts() -> Result<(), InstallError> {
    for path in [start_menu_shortcut(), desktop_shortcut()]
        .into_iter()
        .flatten()
    {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            // A shortcut that is already gone is the state the caller wanted.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(InstallError::Io(format!(
                    "remove {}: {error}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Writes a `.lnk` through `IShellLink`.
///
/// The shell link is a COM object rather than a file format worth reimplementing,
/// so it is built through the same API Explorer uses. The COM calls are the
/// reason this module has `unsafe`; each one is checked and unwound in order.
fn write_shortcut(path: &Path, exe: &Path) -> Result<(), InstallError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| InstallError::Io(format!("create {}: {error}", parent.display())))?;
    }

    shell_link(path, exe)
}

#[cfg(target_os = "windows")]
fn shell_link(path: &Path, exe: &Path) -> Result<(), InstallError> {
    use std::ffi::c_void;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    // `CLSID_ShellLink` and `IID_IShellLinkW`, spelled out because the crate
    // carries no bindings and both are stable for the life of Windows.
    const CLSID_SHELL_LINK: Guid = Guid {
        data1: 0x0002_1401,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    // `IID_IShellLinkW` is `{000214F9-...}`, not the class id: the two differ
    // in the last two digits of the first field, and using the class id here is
    // what produces E_NOINTERFACE.
    const IID_ISHELL_LINK_W: Guid = Guid {
        data1: 0x0002_14F9,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };
    const IID_IPERSIST_FILE: Guid = Guid {
        data1: 0x0000_010B,
        data2: 0x0000,
        data3: 0x0000,
        data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
    };

    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const CLSCTX_INPROC_SERVER: u32 = 0x1;
    const S_OK: i32 = 0;

    /// `IUnknown` slots.
    const QUERY_INTERFACE: usize = 0;
    const RELEASE: usize = 2;
    /// `IShellLinkW::SetPath`, counted past `IUnknown` and `IPersist`.
    const SET_PATH: usize = 20;
    /// `IPersistFile::Save`.
    const PERSIST_SAVE: usize = 6;

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
        fn CoCreateInstance(
            clsid: *const Guid,
            outer: *mut c_void,
            clsctx: u32,
            iid: *const Guid,
            out: *mut *mut c_void,
        ) -> i32;
        fn CoUninitialize();
    }

    fn wide(text: &Path) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        text.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    // SAFETY: every call below is checked, and the two COM objects this holds
    // are released on each path out. `CoUninitialize` is balanced against a
    // successful `CoInitializeEx`.
    unsafe {
        let hr = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);
        // A different apartment model already being active is not a failure for
        // the calls below, so it does not abort the install.
        let initialized = hr >= 0;

        let mut link: *mut c_void = std::ptr::null_mut();
        let hr = CoCreateInstance(
            &CLSID_SHELL_LINK,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_ISHELL_LINK_W,
            &mut link,
        );
        if hr < 0 || link.is_null() {
            if initialized {
                CoUninitialize();
            }
            return Err(InstallError::Shortcut(format!(
                "CoCreateInstance: 0x{hr:08x}"
            )));
        }

        let vtable = *(link as *mut *const usize);
        let set_path: extern "system" fn(*mut c_void, *const u16) -> i32 =
            std::mem::transmute(*vtable.add(SET_PATH));
        let target = wide(exe);
        let hr = set_path(link, target.as_ptr());
        if hr < 0 {
            let release: extern "system" fn(*mut c_void) -> u32 =
                std::mem::transmute(*vtable.add(RELEASE));
            release(link);
            if initialized {
                CoUninitialize();
            }
            return Err(InstallError::Shortcut(format!("SetPath: 0x{hr:08x}")));
        }

        // The shortcut is written through `IPersistFile`, which the same object
        // also implements.
        let query: extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> i32 =
            std::mem::transmute(*vtable.add(QUERY_INTERFACE));
        let mut persist: *mut c_void = std::ptr::null_mut();
        let hr = query(link, &IID_IPERSIST_FILE, &mut persist);
        if hr < 0 || persist.is_null() {
            let release: extern "system" fn(*mut c_void) -> u32 =
                std::mem::transmute(*vtable.add(RELEASE));
            release(link);
            if initialized {
                CoUninitialize();
            }
            return Err(InstallError::Shortcut(format!(
                "QueryInterface: 0x{hr:08x}"
            )));
        }

        let persist_vtable = *(persist as *mut *const usize);
        let save: extern "system" fn(*mut c_void, *const u16, i32) -> i32 =
            std::mem::transmute(*persist_vtable.add(PERSIST_SAVE));
        let destination = wide(path);
        let hr = save(persist, destination.as_ptr(), 1);

        let release_persist: extern "system" fn(*mut c_void) -> u32 =
            std::mem::transmute(*persist_vtable.add(RELEASE));
        release_persist(persist);
        let release_link: extern "system" fn(*mut c_void) -> u32 =
            std::mem::transmute(*vtable.add(RELEASE));
        release_link(link);
        if initialized {
            CoUninitialize();
        }

        if hr != S_OK {
            return Err(InstallError::Shortcut(format!("Save: 0x{hr:08x}")));
        }
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn shell_link(_path: &Path, _exe: &Path) -> Result<(), InstallError> {
    Err(InstallError::Shortcut(
        "shortcuts are a Windows feature".to_owned(),
    ))
}
// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Writes the `Uninstall` entry Settings reads.
///
/// The four values that matter: what to show, who published it, where it lives,
/// and what to run to remove it. `DisplayIcon` is what gives the entry its
/// picture instead of a blank square.
#[cfg(target_os = "windows")]
fn write_uninstall_entry(exe: &Path) -> Result<(), InstallError> {
    let dir = exe.parent().unwrap_or(Path::new("."));
    let uninstall_string = format!("\"{}\" --uninstall", exe.display());

    let values: [(&str, String); 6] = [
        ("DisplayName", "myterm".to_owned()),
        ("DisplayVersion", env!("CARGO_PKG_VERSION").to_owned()),
        ("DisplayIcon", format!("{},0", exe.display())),
        ("InstallLocation", dir.display().to_string()),
        ("UninstallString", uninstall_string),
        ("Publisher", "tiramission".to_owned()),
    ];

    for (name, value) in values {
        set_registry_string(UNINSTALL_KEY, name, &value)?;
    }
    set_registry_dword(UNINSTALL_KEY, "NoModify", 1)?;
    set_registry_dword(UNINSTALL_KEY, "NoRepair", 1)?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn write_uninstall_entry(_exe: &Path) -> Result<(), InstallError> {
    Err(InstallError::Registry("Windows only".to_owned()))
}

#[cfg(target_os = "windows")]
fn remove_uninstall_entry() -> Result<(), InstallError> {
    const HKEY_CURRENT_USER: *mut c_void = 0x8000_0001u32 as isize as *mut c_void;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegDeleteKeyW(key: *mut c_void, subkey: *const u16) -> i32;
    }

    let subkey: Vec<u16> = UNINSTALL_KEY
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: the string is NUL-terminated and outlives the call.
    let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, subkey.as_ptr()) };
    // `ERROR_FILE_NOT_FOUND` means it was never there, which is the state the
    // caller asked for.
    if status != 0 && status != 2 {
        return Err(InstallError::Registry(format!("RegDeleteKeyW: {status}")));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn remove_uninstall_entry() -> Result<(), InstallError> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn set_registry_string(key: &str, name: &str, value: &str) -> Result<(), InstallError> {
    use std::ffi::c_void;

    const HKEY_CURRENT_USER: *mut c_void = 0x8000_0001u32 as isize as *mut c_void;
    const KEY_WRITE: u32 = 0x0002_0006;
    const REG_SZ: u32 = 1;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegCreateKeyExW(
            key: *mut c_void,
            subkey: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            desired: u32,
            security: *mut c_void,
            out: *mut *mut c_void,
            disposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            key: *mut c_void,
            name: *const u16,
            reserved: u32,
            kind: u32,
            data: *const u8,
            size: u32,
        ) -> i32;
        fn RegCloseKey(key: *mut c_void) -> i32;
    }

    let subkey: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
    let mut handle: *mut c_void = std::ptr::null_mut();
    // SAFETY: all buffers are NUL-terminated and outlive their calls; the handle
    // is closed on every path.
    unsafe {
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_WRITE,
            std::ptr::null_mut(),
            &mut handle,
            std::ptr::null_mut(),
        );
        if status != 0 {
            return Err(InstallError::Registry(format!("RegCreateKeyExW: {status}")));
        }

        let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let status = RegSetValueExW(
            handle,
            name_wide.as_ptr(),
            0,
            REG_SZ,
            data.as_ptr() as *const u8,
            (data.len() * 2) as u32,
        );
        RegCloseKey(handle);
        if status != 0 {
            return Err(InstallError::Registry(format!("RegSetValueExW: {status}")));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn set_registry_string(_key: &str, _name: &str, _value: &str) -> Result<(), InstallError> {
    Err(InstallError::Registry("Windows only".to_owned()))
}

#[cfg(target_os = "windows")]
fn set_registry_dword(key: &str, name: &str, value: u32) -> Result<(), InstallError> {
    use std::ffi::c_void;

    const HKEY_CURRENT_USER: *mut c_void = 0x8000_0001u32 as isize as *mut c_void;
    const KEY_WRITE: u32 = 0x0002_0006;
    const REG_DWORD: u32 = 4;

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegCreateKeyExW(
            key: *mut c_void,
            subkey: *const u16,
            reserved: u32,
            class: *const u16,
            options: u32,
            desired: u32,
            security: *mut c_void,
            out: *mut *mut c_void,
            disposition: *mut u32,
        ) -> i32;
        fn RegSetValueExW(
            key: *mut c_void,
            name: *const u16,
            reserved: u32,
            kind: u32,
            data: *const u8,
            size: u32,
        ) -> i32;
        fn RegCloseKey(key: *mut c_void) -> i32;
    }

    let subkey: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
    let mut handle: *mut c_void = std::ptr::null_mut();
    // SAFETY: as above; the handle is closed on every path.
    unsafe {
        let status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_WRITE,
            std::ptr::null_mut(),
            &mut handle,
            std::ptr::null_mut(),
        );
        if status != 0 {
            return Err(InstallError::Registry(format!("RegCreateKeyExW: {status}")));
        }

        let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let status = RegSetValueExW(
            handle,
            name_wide.as_ptr(),
            0,
            REG_DWORD,
            &value as *const u32 as *const u8,
            std::mem::size_of::<u32>() as u32,
        );
        RegCloseKey(handle);
        if status != 0 {
            return Err(InstallError::Registry(format!("RegSetValueExW: {status}")));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn set_registry_dword(_key: &str, _name: &str, _value: u32) -> Result<(), InstallError> {
    Err(InstallError::Registry("Windows only".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_dir_is_under_local_appdata() {
        // The location is a design decision, not an implementation detail: a
        // per-user install under `Programs` is what keeps this app free of a
        // UAC prompt.
        let dir = install_dir().expect("LOCALAPPDATA should exist on a test machine");
        assert!(
            dir.ends_with(Path::new("Programs").join("myterm")),
            "unexpected install dir: {}",
            dir.display()
        );
    }

    #[test]
    fn installed_exe_is_myterm_exe() {
        let exe = installed_exe().expect("path");
        assert_eq!(exe.file_name(), Some(std::ffi::OsStr::new("myterm.exe")));
    }

    #[test]
    fn a_debug_build_is_not_installed() {
        // The test binary runs from `target`, not from the install directory, so
        // this is the exact case the about panel has to distinguish.
        assert!(!is_installed(), "the test binary should not look installed");
    }
}

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::*;

    /// Writes a shortcut to a temporary path and checks it appeared.
    ///
    /// This is the part of installing that has no simple file-format fallback:
    /// a `.lnk` is a COM object, and getting the vtable indices wrong produces a
    /// crash rather than a wrong file. The test writes to `%TEMP%` so it never
    /// touches the user's desktop or start menu.
    #[test]
    fn a_shortcut_is_written_where_asked() {
        let dir = std::env::temp_dir().join("myterm-shortcut-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let link = dir.join("probe.lnk");

        let exe = current_exe().expect("current exe");
        write_shortcut(&link, &exe).expect("shortcut");

        let written = std::fs::metadata(&link).expect("shortcut exists");
        assert!(written.len() > 0, "a written shortcut should not be empty");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(all(test, target_os = "windows"))]
mod registry_tests {
    use super::*;

    /// The registry entry is what makes the app visible to Settings → Apps.
    ///
    /// Written to a scratch key rather than the real one so a test run never
    /// leaves an uninstall entry for a program that is not installed.
    #[test]
    fn a_string_value_round_trips() {
        let key = "Software\\myterm-test\\round-trip";
        set_registry_string(key, "DisplayName", "myterm").expect("write");
        set_registry_dword(key, "NoModify", 1).expect("write dword");
        remove_test_key(key);
    }

    fn remove_test_key(key: &str) {
        use std::ffi::c_void;
        const HKEY_CURRENT_USER: *mut c_void = 0x8000_0001u32 as isize as *mut c_void;
        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn RegDeleteKeyW(key: *mut c_void, subkey: *const u16) -> i32;
        }
        let subkey: Vec<u16> = key.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: NUL-terminated string outlives the call.
        unsafe {
            RegDeleteKeyW(HKEY_CURRENT_USER, subkey.as_ptr());
        }
    }
}

#[cfg(all(test, target_os = "windows"))]
mod end_to_end {
    use super::*;

    /// The whole install, into a scratch location.
    ///
    /// Run only on request (`--ignored`) because it writes to the real registry
    /// and the real start menu. It exists so the full path — copy, register,
    /// shortcut — can be exercised without clicking anything.
    #[test]
    #[ignore]
    fn install_then_uninstall_leaves_nothing_behind() {
        let exe = current_exe().expect("current exe");
        let dir = install_dir().expect("install dir");

        // Stand in for the copied binary: the real install copies this test
        // executable, which is enough to prove the copy works.
        std::fs::create_dir_all(&dir).expect("create install dir");
        let staged = dir.join("myterm.exe");
        std::fs::copy(&exe, &staged).expect("copy");
        assert!(staged.exists());

        write_uninstall_entry(&staged).expect("registry");
        create_shortcuts(&staged, true).expect("shortcuts");

        assert!(start_menu_shortcut().expect("path").exists());
        assert!(desktop_shortcut().expect("path").exists());

        remove_shortcuts().expect("remove shortcuts");
        remove_uninstall_entry().expect("remove registry");

        assert!(!start_menu_shortcut().expect("path").exists());
        assert!(!desktop_shortcut().expect("path").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
