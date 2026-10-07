//! Minimal HTTPS client on top of WinHTTP: uses the OS TLS stack and proxy settings,
//! so no TLS library is linked into the binary.

use crate::util::wide;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Networking::WinHttp::*;

pub struct Resp {
    pub status: u32,
    pub body: String,
}

struct H(*mut c_void);
impl Drop for H {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

pub fn get(url: &str, headers: &[(&str, &str)], follow: bool) -> Result<Resp, String> {
    request("GET", url, headers, &[], follow)
}

pub fn post_json(url: &str, body: &str) -> Result<Resp, String> {
    request("POST", url, &[("Content-Type", "application/json")], body.as_bytes(), false)
}

fn request(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8], follow: bool) -> Result<Resp, String> {
    let rest = url.strip_prefix("https://").ok_or("https only")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (ua, host_w, method_w, path_w) = (wide("llm-tracker/0.1"), wide(host), wide(method), wide(path));
    let mut hdr = String::new();
    for (k, v) in headers {
        hdr.push_str(&format!("{k}: {v}\r\n"));
    }
    let hdr_w = wide(&hdr);

    unsafe {
        let ses = H(WinHttpOpen(ua.as_ptr(), WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, null(), null(), 0));
        if ses.0.is_null() {
            return Err("WinHttpOpen failed".into());
        }
        WinHttpSetTimeouts(ses.0, 10_000, 10_000, 15_000, 20_000);
        let dec: u32 = WINHTTP_DECOMPRESSION_FLAG_GZIP | WINHTTP_DECOMPRESSION_FLAG_DEFLATE;
        WinHttpSetOption(ses.0, WINHTTP_OPTION_DECOMPRESSION, &dec as *const u32 as *const c_void, 4);

        let con = H(WinHttpConnect(ses.0, host_w.as_ptr(), 443, 0));
        if con.0.is_null() {
            return Err("네트워크 연결 실패".into());
        }
        let req = H(WinHttpOpenRequest(
            con.0,
            method_w.as_ptr(),
            path_w.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err("WinHttpOpenRequest failed".into());
        }
        if !follow {
            let pol: u32 = WINHTTP_OPTION_REDIRECT_POLICY_NEVER;
            WinHttpSetOption(req.0, WINHTTP_OPTION_REDIRECT_POLICY, &pol as *const u32 as *const c_void, 4);
        }

        let (bptr, blen) = if body.is_empty() { (null(), 0) } else { (body.as_ptr() as *const c_void, body.len() as u32) };
        if WinHttpSendRequest(req.0, hdr_w.as_ptr(), u32::MAX, bptr, blen, blen, 0) == 0
            || WinHttpReceiveResponse(req.0, null_mut()) == 0
        {
            return Err("네트워크 오류".into());
        }

        let mut status: u32 = 0;
        let mut size: u32 = 4;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as *mut c_void,
            &mut size,
            null_mut(),
        );

        let mut data = Vec::new();
        loop {
            let mut avail: u32 = 0;
            if WinHttpQueryDataAvailable(req.0, &mut avail) == 0 || avail == 0 {
                break;
            }
            let start = data.len();
            data.resize(start + avail as usize, 0);
            let mut read: u32 = 0;
            if WinHttpReadData(req.0, data[start..].as_mut_ptr() as *mut c_void, avail, &mut read) == 0 {
                break;
            }
            data.truncate(start + read as usize);
            if data.len() > 8 << 20 {
                break;
            }
        }
        Ok(Resp { status, body: String::from_utf8_lossy(&data).into_owned() })
    }
}
