//! Attachments as files the computer's own protection checks. Merel has no virus scanner of
//! its own: one built in would never be as current as the one the person installed, and would
//! itself be code that reads hostile files. Instead every attachment Merel writes is an
//! ordinary file with the label a browser gives a download, so the protection that is there
//! treats it as coming from the internet:
//!
//! - macOS: the `com.apple.quarantine` attribute, so Gatekeeper and XProtect check it when it
//!   is opened, and a program in it asks before it runs.
//! - Windows: the Mark of the Web (`Zone.Identifier`, internet zone), so SmartScreen warns
//!   and Office opens it in Protected View; and before the file is written the installed
//!   virus scanner (Microsoft Defender or another) looks at it through AMSI.
//! - Linux and the rest: no such label exists; a scanner that watches the disk still sees the
//!   file, as it is a plain file.

use std::path::Path;

/// What the installed virus scanner said about an attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The scanner looked and found nothing.
    Clean,
    /// The scanner found malware, or the administrator blocks this content.
    Malware,
    /// No scanner could be asked: another platform, none installed, or it failed.
    Unscanned,
}

/// The name macOS shows as the program that downloaded the file.
pub const AGENT: &str = "Merel";

/// The value of `com.apple.quarantine` for a file Merel wrote at `unix_secs`: flags, time in
/// hexadecimal, the program, and an empty event id, as browsers and Mail write it.
pub fn quarantine_value(unix_secs: u64) -> String {
    format!("0081;{unix_secs:08x};{AGENT};")
}

/// The contents of the `Zone.Identifier` stream: zone 3 is the internet.
pub const ZONE_IDENTIFIER: &str = "[ZoneTransfer]\r\nZoneId=3\r\n";

/// Labels the file at `path` as downloaded from the internet. A disk that cannot hold the label
/// (a USB stick in FAT, a network share) is no reason to fail the save; the error is returned
/// for the log.
pub fn mark(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        macos::set_quarantine(path, &quarantine_value(secs))
    }
    #[cfg(windows)]
    {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
        std::fs::write(stream, ZONE_IDENTIFIER)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = path;
        Ok(())
    }
}

/// Asks the installed virus scanner about `bytes`, the attachment `name`. Only Windows offers
/// a way to ask (AMSI); elsewhere the answer is [`Verdict::Unscanned`] and the label from
/// [`mark`] does the work when the file is opened.
pub fn scan(name: &str, bytes: &[u8]) -> Verdict {
    #[cfg(windows)]
    {
        windows::scan(name, bytes)
    }
    #[cfg(not(windows))]
    {
        let _ = (name, bytes);
        Verdict::Unscanned
    }
}

/// Whether this computer has a way to ask a virus scanner before a file is written.
pub const CAN_SCAN: bool = cfg!(windows);

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;
    use std::path::Path;

    pub fn set_quarantine(path: &Path, value: &str) -> std::io::Result<()> {
        let c_path = CString::new(path.as_os_str().as_bytes()).map_err(std::io::Error::other)?;
        let name = c"com.apple.quarantine";
        // SAFETY: both strings are NUL-terminated and live through the call; the value pointer
        // and length describe one live byte slice; position 0 and options 0 are what setxattr
        // expects for an ordinary attribute.
        let rc = unsafe { libc::setxattr(c_path.as_ptr(), name.as_ptr(), value.as_ptr().cast(), value.len(), 0, 0) };
        if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error()) }
    }
}

#[cfg(windows)]
mod windows {
    use super::Verdict;
    use windows_sys::Win32::System::Antimalware::{
        AMSI_RESULT, AMSI_RESULT_DETECTED, AmsiCloseSession, AmsiInitialize, AmsiOpenSession, AmsiScanBuffer, AmsiUninitialize, HAMSICONTEXT,
        HAMSISESSION,
    };

    /// The range an administrator uses to block content by policy.
    const BLOCKED_BY_ADMIN: std::ops::RangeInclusive<AMSI_RESULT> = 0x4000..=0x4fff;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().filter(|c| *c != 0).chain(std::iter::once(0)).collect()
    }

    pub fn scan(name: &str, bytes: &[u8]) -> Verdict {
        let Ok(len) = u32::try_from(bytes.len()) else { return Verdict::Unscanned };
        let app = wide(super::AGENT);
        let content = wide(name);
        let mut ctx: HAMSICONTEXT = std::ptr::null_mut();
        // SAFETY: `app` is a NUL-terminated UTF-16 string that outlives the call; `ctx` is a
        // valid place for the handle.
        if unsafe { AmsiInitialize(app.as_ptr(), &mut ctx) } < 0 || ctx.is_null() {
            log::info!("no virus scanner to ask (AMSI could not start)");
            return Verdict::Unscanned;
        }
        let mut session: HAMSISESSION = std::ptr::null_mut();
        // SAFETY: `ctx` came from AmsiInitialize above; `session` is a valid place for the handle.
        let opened = unsafe { AmsiOpenSession(ctx, &mut session) } >= 0;
        let mut result: AMSI_RESULT = 0;
        // SAFETY: `bytes` is live for the call and `len` is its length; `content` is a
        // NUL-terminated UTF-16 string; the handles came from the calls above (a null session is
        // allowed); `result` is a valid place for the answer.
        let hr = unsafe {
            AmsiScanBuffer(ctx, bytes.as_ptr().cast(), len, content.as_ptr(), if opened { session } else { std::ptr::null_mut() }, &mut result)
        };
        // SAFETY: the handles came from the calls above and are closed once, session first.
        unsafe {
            if opened {
                AmsiCloseSession(ctx, session);
            }
            AmsiUninitialize(ctx);
        }
        if hr < 0 {
            log::info!("the virus scanner did not answer (AMSI {hr:#x})");
            Verdict::Unscanned
        } else if result >= AMSI_RESULT_DETECTED || BLOCKED_BY_ADMIN.contains(&result) {
            Verdict::Malware
        } else {
            Verdict::Clean
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quarantine_value_names_merel_and_the_time() {
        assert_eq!(quarantine_value(0x6543_21ab), "0081;654321ab;Merel;");
        assert_eq!(quarantine_value(5), "0081;00000005;Merel;");
    }

    #[test]
    fn the_zone_is_the_internet() {
        assert!(ZONE_IDENTIFIER.contains("ZoneId=3"));
    }

    #[test]
    fn marking_a_file_keeps_it_and_its_contents() {
        let dir = std::env::temp_dir().join(format!("merel-quarantine-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("factuur.pdf");
        std::fs::write(&path, b"%PDF").unwrap();
        mark(&path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"%PDF");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_a_way_to_ask_nothing_is_called_clean() {
        if !CAN_SCAN {
            assert_eq!(scan("a.exe", b"MZ"), Verdict::Unscanned);
        }
    }
}
