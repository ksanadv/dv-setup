// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dv-subsystem-set contributors
//! The dv apps' GitHub releases over WinHTTP: the latest version of a repo, and its setup program downloaded and
//! checked against the SHA-256 published next to it. Release files are named `<app>-<version>-windows-x64-setup.exe`.
use std::ffi::c_void;
use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

const HOST: &str = "github.com";
const MAX_DOWNLOAD: usize = 64 << 20;

/// a release's setup program, "dvnote-0.3.2-windows-x64-setup.exe"
pub fn setup_name(app: &str, version: &str) -> String {
    format!("{app}-{version}-windows-x64-setup.exe")
}

/// The latest released version of `repo` ("ksanadv/dvnote"), or None when it has no release yet. One request with
/// redirects off: github.com answers /releases/latest with a redirect to /releases/tag/<tag>, and with one to
/// /releases when there is none.
pub fn latest(repo: &str, agent: &str) -> Result<Option<String>, String> {
    let r = Request::get(&format!("/{repo}/releases/latest"), false, agent)?;
    if let Some(v) = r.header(WINHTTP_QUERY_LOCATION).and_then(|l| version_from_location(&l)) {
        return Ok(Some(v));
    }
    match r.status() {
        Some(200 | 302 | 404) => Ok(None),
        Some(s) => Err(format!("{HOST} answered with HTTP status {s}.")),
        None => Err(format!("{HOST} sent no answer.")),
    }
}

/// `app`'s setup program for `version`, checked against its published SHA-256; `progress` hears how far the download
/// is (0 to 1)
pub fn download(repo: &str, app: &str, version: &str, agent: &str, progress: &dyn Fn(f32)) -> Result<Vec<u8>, String> {
    let base = format!("/{repo}/releases/download/v{version}/{}", setup_name(app, version));
    let setup = Request::get(&base, true, agent)?.body(&format!("{app} {version}"), progress)?;
    let sums = Request::get(&format!("{base}.sha256"), true, agent)?.body("its checksum", &|_| {})?;
    let expected = parse_sha256(&String::from_utf8_lossy(&sums)).ok_or("Its checksum file is not valid.")?;
    if sha256(&setup) != Some(expected) {
        return Err(format!("{app} {version} does not match its published checksum, so it was not installed."));
    }
    Ok(setup)
}

fn version_from_location(location: &str) -> Option<String> {
    let tag = location.rsplit_once("/releases/tag/")?.1;
    let v = tag.strip_prefix('v').unwrap_or(tag);
    let valid = !v.is_empty() && v.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    valid.then(|| v.to_string())
}

/// the first token of a `sha256sum`-style line
fn parse_sha256(text: &str) -> Option<[u8; 32]> {
    let hex = text.trim_start_matches('\u{feff}').split_whitespace().next()?.as_bytes();
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in hex.chunks(2).enumerate() {
        out[i] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

fn sha256(data: &[u8]) -> Option<[u8; 32]> {
    use windows::Win32::Security::Cryptography::{BCryptHash, BCRYPT_SHA256_ALG_HANDLE};
    let mut out = [0u8; 32];
    unsafe { BCryptHash(BCRYPT_SHA256_ALG_HANDLE, None, data, &mut out) }.is_ok().then_some(out)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

// fields drop in order: request, connection, session
struct Request {
    req: Handle,
    _connect: Handle,
    _session: Handle,
}

fn failed(what: &str) -> String {
    format!("Could not {what} {HOST} ({}). Check the internet connection.", std::io::Error::last_os_error())
}

impl Request {
    fn get(path: &str, follow_redirects: bool, agent: &str) -> Result<Request, String> {
        let (agent, host, path) = (wide(agent), wide(HOST), wide(path));
        unsafe {
            let session = Handle(WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            ));
            if session.0.is_null() {
                return Err(failed("reach"));
            }
            let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 30_000);
            let connect = Handle(WinHttpConnect(session.0, PCWSTR(host.as_ptr()), INTERNET_DEFAULT_HTTPS_PORT, 0));
            if connect.0.is_null() {
                return Err(failed("reach"));
            }
            let req = Handle(WinHttpOpenRequest(
                connect.0,
                w!("GET"),
                PCWSTR(path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            ));
            if req.0.is_null() {
                return Err(failed("reach"));
            }
            let policy = if follow_redirects {
                WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP
            } else {
                WINHTTP_OPTION_REDIRECT_POLICY_NEVER
            };
            let _ = WinHttpSetOption(Some(req.0), WINHTTP_OPTION_REDIRECT_POLICY, Some(&policy.to_ne_bytes()));
            if WinHttpSendRequest(req.0, None, None, 0, 0, 0).is_err()
                || WinHttpReceiveResponse(req.0, std::ptr::null_mut()).is_err()
            {
                return Err(failed("connect to"));
            }
            Ok(Request { req, _connect: connect, _session: session })
        }
    }

    fn number(&self, which: u32) -> Option<u32> {
        let (mut n, mut len) = (0u32, 4u32);
        let flags = which | WINHTTP_QUERY_FLAG_NUMBER;
        let buf = Some(&mut n as *mut u32 as *mut c_void);
        unsafe { WinHttpQueryHeaders(self.req.0, flags, PCWSTR::null(), buf, &mut len, std::ptr::null_mut()) }
            .ok()
            .map(|()| n)
    }

    fn status(&self) -> Option<u32> {
        self.number(WINHTTP_QUERY_STATUS_CODE)
    }

    fn header(&self, which: u32) -> Option<String> {
        let mut buf = [0u16; 2048];
        let mut len = (buf.len() * 2) as u32;
        let ptr = Some(buf.as_mut_ptr() as *mut c_void);
        unsafe { WinHttpQueryHeaders(self.req.0, which, PCWSTR::null(), ptr, &mut len, std::ptr::null_mut()) }
            .ok()
            .map(|()| String::from_utf16_lossy(&buf[..len as usize / 2]))
    }

    fn body(self, what: &str, progress: &dyn Fn(f32)) -> Result<Vec<u8>, String> {
        match self.status() {
            Some(200) => {}
            Some(s) => return Err(format!("Could not download {what}: {HOST} answered with HTTP status {s}.")),
            None => return Err(failed("download from")),
        }
        let total = self.number(WINHTTP_QUERY_CONTENT_LENGTH).unwrap_or(0) as usize;
        let mut out = Vec::with_capacity(total.min(MAX_DOWNLOAD));
        let mut chunk = vec![0u8; 64 << 10];
        loop {
            let mut read = 0u32;
            let buf = chunk.as_mut_ptr() as *mut c_void;
            if unsafe { WinHttpReadData(self.req.0, buf, chunk.len() as u32, &mut read) }.is_err() {
                return Err(failed("download from"));
            }
            if read == 0 {
                return Ok(out);
            }
            out.extend_from_slice(&chunk[..read as usize]);
            if out.len() > MAX_DOWNLOAD {
                return Err(format!("{what} is larger than expected, so it was not installed."));
            }
            if total > 0 {
                progress(out.len() as f32 / total as f32);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn location_to_version() {
        let v = |s: &str| version_from_location(s);
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/v0.4.1").as_deref(), Some("0.4.1"));
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/1.2").as_deref(), Some("1.2"));
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases"), None);
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/v1.0-beta"), None);
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/v1..2"), None);
        assert_eq!(v("https://github.com/ksanadv/dvnote/releases/tag/v"), None);
    }

    #[test]
    fn checksum_file() {
        let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let empty = sha256(b"").unwrap();
        assert_eq!(parse_sha256(&format!("{hex}  dvnote-1.2.3-windows-x64-setup.exe\n")), Some(empty));
        assert_eq!(parse_sha256(&hex.to_uppercase()), Some(empty));
        assert_eq!(parse_sha256(&format!("\u{feff}{hex}  dvnote-1.2.3-windows-x64-setup.exe")), Some(empty));
        assert_eq!(parse_sha256("abc  dvnote-1.2.3-windows-x64-setup.exe"), None);
        assert_eq!(parse_sha256(""), None);
    }

    /// DV_NETTEST=1: the real releases, downloaded and checked, never run
    #[test]
    fn real_releases() {
        if std::env::var_os("DV_NETTEST").is_none() {
            return;
        }
        let v = latest("ksanadv/dvsnip", "dv-setup-test").unwrap().expect("dvsnip has a release");
        let setup = download("ksanadv/dvsnip", "dvsnip", &v, "dv-setup-test", &|_| {}).unwrap();
        assert_eq!(&setup[..2], b"MZ");
    }
}
