//! Thin, safe wrappers around the Windows APIs Fetchbox needs.

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::io::FromRawHandle;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};
use windows_sys::Win32::Security::{GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegEnumKeyExW, RegOpenKeyExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn from_wide_ptr(p: *const u16) -> String {
    let mut len = 0;
    while *p.add(len) != 0 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

/// A `Command` that never flashes a console window.
pub fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// Start a program that outlives us (and the browser's job object, when we are the browser helper).
pub fn spawn_detached(exe: &Path, args: &[&str]) -> io::Result<()> {
    let run = |flags: u32| {
        Command::new(exe)
            .args(args)
            .current_dir(exe.parent().unwrap_or(Path::new(".")))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
    };
    let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    run(base | CREATE_BREAKAWAY_FROM_JOB).or_else(|_| run(base)).map(|_| ())
}

/// Kill a process and everything it started (yt-dlp → ffmpeg, node → workers).
pub fn kill_tree(pid: u32) {
    let _ = quiet_command("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn process_is(pid: u32, image: &str) -> bool {
    quiet_command("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase().contains(&format!("\"{}\"", image.to_ascii_lowercase())))
        .unwrap_or(false)
}

/// Open a web page in the default browser (https only).
pub fn open_web(url: &str) {
    if url.starts_with("https://") && crate::util::valid_url(url) {
        let _ = Command::new("explorer").arg(url).spawn();
    }
}

/// Show a file in Explorer (or open the folder if the file is gone).
pub fn reveal(path: &Path) {
    if path.is_file() {
        let _ = Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    } else {
        let dir = if path.is_dir() { path } else { path.parent().unwrap_or(path) };
        let _ = Command::new("explorer").arg(dir).spawn();
    }
}

// ── current user ──
fn current_user_sid() -> io::Result<String> {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut buf = vec![0u8; 512];
        let mut len = 0u32;
        let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut len);
        CloseHandle(token);
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            return Err(io::Error::last_os_error());
        }
        let sid = from_wide_ptr(s);
        LocalFree(s as *mut c_void);
        Ok(sid)
    }
}

// ── private named pipe ──
/// One listening instance of a pipe that only the current user (and SYSTEM) can open, local machine only.
pub struct PipeListener {
    name: Vec<u16>,
    sddl: Vec<u16>,
    next: HANDLE,
}
unsafe impl Send for PipeListener {}

impl PipeListener {
    /// Fails with `AlreadyExists` if another process already serves this pipe.
    pub fn bind(name: &str) -> io::Result<Self> {
        let sid = current_user_sid()?;
        let mut l = PipeListener {
            name: wide(&format!(r"\\.\pipe\{name}")),
            sddl: wide(&format!("D:P(A;;GA;;;{sid})(A;;GA;;;SY)")),
            next: null_mut(),
        };
        l.next = l.create(true)?;
        Ok(l)
    }

    fn create(&self, first: bool) -> io::Result<HANDLE> {
        unsafe {
            let mut sd: *mut c_void = null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(self.sddl.as_ptr(), 1, &mut sd, null_mut()) == 0 {
                return Err(io::Error::last_os_error());
            }
            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: 0,
            };
            let h = CreateNamedPipeW(
                self.name.as_ptr(),
                PIPE_ACCESS_DUPLEX | if first { FILE_FLAG_FIRST_PIPE_INSTANCE } else { 0 },
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                &sa,
            );
            let err = io::Error::last_os_error();
            LocalFree(sd);
            if h == INVALID_HANDLE_VALUE {
                return Err(if first { io::Error::new(io::ErrorKind::AlreadyExists, err) } else { err });
            }
            Ok(h)
        }
    }

    /// Wait for the next client.
    pub fn accept(&mut self) -> io::Result<File> {
        unsafe {
            let h = self.next;
            if ConnectNamedPipe(h, null_mut()) == 0 && GetLastError() != ERROR_PIPE_CONNECTED {
                let e = io::Error::last_os_error();
                CloseHandle(h);
                self.next = self.create(false)?;
                return Err(e);
            }
            self.next = self.create(false)?;
            Ok(File::from_raw_handle(h as _))
        }
    }
}

// ── secrets at rest (Windows DPAPI: only this Windows user can decrypt) ──
pub fn protect(plain: &[u8]) -> Option<Vec<u8>> {
    dpapi(plain, true)
}
pub fn unprotect(cipher: &[u8]) -> Option<Vec<u8>> {
    dpapi(cipher, false)
}
fn dpapi(data: &[u8], encrypt: bool) -> Option<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
        let ok = if encrypt {
            CryptProtectData(&input, null(), null(), null(), null(), 0, &mut out)
        } else {
            CryptUnprotectData(&input, null_mut(), null(), null(), null(), 0, &mut out)
        };
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as *mut c_void);
        Some(v)
    }
}

// ── registry (current user only) ──
pub fn reg_set_default(subkey: &str, value: &str) -> io::Result<()> {
    unsafe {
        let mut key: HKEY = null_mut();
        let rc = RegCreateKeyExW(HKEY_CURRENT_USER, wide(subkey).as_ptr(), 0, null(), 0, KEY_WRITE, null(), &mut key, null_mut());
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        let data = wide(value);
        let rc = RegSetValueExW(key, null(), 0, REG_SZ, data.as_ptr() as *const u8, (data.len() * 2) as u32);
        RegCloseKey(key);
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        Ok(())
    }
}

pub fn reg_delete(subkey: &str) -> bool {
    unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, wide(subkey).as_ptr()) == 0 }
}

pub fn reg_subkeys(subkey: &str) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut key: HKEY = null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, wide(subkey).as_ptr(), 0, KEY_READ, &mut key) != 0 {
            return out;
        }
        for i in 0.. {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(key, i, name.as_mut_ptr(), &mut len, null(), null_mut(), null_mut(), null_mut()) != 0 {
                break;
            }
            out.push(String::from_utf16_lossy(&name[..len as usize]));
        }
        RegCloseKey(key);
    }
    out
}

// ── single window ──
/// True if this is the only Fetchbox window. The mutex lives until the process exits.
pub fn claim_single_instance(name: &str) -> bool {
    unsafe {
        let h = CreateMutexW(null(), 0, wide(name).as_ptr());
        !(h.is_null() || GetLastError() == ERROR_ALREADY_EXISTS)
    }
}

pub fn focus_window(title: &str) -> bool {
    unsafe {
        let hwnd = FindWindowW(null(), wide(title).as_ptr());
        if hwnd.is_null() {
            return false;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd) != 0
    }
}

/// Map a file read-only for the rest of the program's life. Its pages stay backed by the file, so they
/// don't count as this program's memory and only the parts in use are loaded (the window's fonts).
pub fn map_file_forever(path: &Path) -> Option<&'static [u8]> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Memory::{CreateFileMappingW, MapViewOfFile, FILE_MAP_READ, PAGE_READONLY};
    let file = File::open(path).ok()?;
    let len = usize::try_from(file.metadata().ok()?.len()).ok().filter(|&n| n > 0)?;
    unsafe {
        let map = CreateFileMappingW(file.as_raw_handle() as HANDLE, null(), PAGE_READONLY, 0, 0, null());
        if map.is_null() {
            return None;
        }
        let view = MapViewOfFile(map, FILE_MAP_READ, 0, 0, 0);
        CloseHandle(map); // the view keeps the mapping (and the file) open
        if view.Value.is_null() {
            return None;
        }
        Some(std::slice::from_raw_parts(view.Value as *const u8, len))
    }
}
