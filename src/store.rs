//! Config file (%APPDATA%\llm-tracker\config.json). The OpenCode session cookie is kept
//! encrypted with DPAPI, so it is only readable by this Windows user.

use crate::util::{b64_decode, b64_encode, read_json, write_json_atomic};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

#[derive(Clone)]
pub struct Config {
    pub interval_min: u64,
    pub auto_refresh: bool,
    pub autorun: bool,
    pub cookie: String,
    pub codex_home: String,
}

static LAST_GOOD: Mutex<Option<Config>> = Mutex::new(None);

pub fn path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default();
    base.join("llm-tracker").join("config.json")
}

/// Ok(None): the file does not exist. A file that exists but cannot be read or parsed (an editor
/// is saving it, antivirus holds it, a typo) is retried, then reported as an error.
fn read_config() -> Result<Option<Value>, String> {
    let p = path();
    let mut last = String::new();
    for attempt in 0..4 {
        if !p.exists() {
            return Ok(None);
        }
        match read_json(&p) {
            Ok(v) => return Ok(Some(v)),
            Err(e) => last = e,
        }
        if attempt < 3 {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }
    Err(last)
}

fn parse(v: &Value) -> Config {
    let cookie = v["opencode_cookie_dpapi"]
        .as_str()
        .and_then(b64_decode)
        .and_then(|b| unprotect(&b))
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default();
    Config {
        interval_min: v["interval_min"].as_u64().unwrap_or(5).clamp(1, 120),
        auto_refresh: v["auto_refresh"].as_bool().unwrap_or(false),
        autorun: v["autorun"].as_bool().unwrap_or(false),
        cookie,
        codex_home: v["codex_home"].as_str().unwrap_or("").to_string(),
    }
}

/// Never falls back to defaults because of a transient read failure: that would silently turn
/// off auto_refresh and drop the cookie. It keeps the last good config instead.
pub fn load() -> Config {
    let mut last = LAST_GOOD.lock().unwrap_or_else(|e| e.into_inner());
    match read_config() {
        Ok(v) => {
            let c = parse(&v.unwrap_or(Value::Null));
            *last = Some(c.clone());
            c
        }
        Err(_) => last.clone().unwrap_or_else(|| parse(&Value::Null)),
    }
}

fn update(key: &str, val: Value) -> Result<(), String> {
    let p = path();
    // if the existing file cannot be read, refuse to write: starting from defaults would erase it
    let mut v = match read_config() {
        Ok(Some(v)) => v,
        Ok(None) => json!({"interval_min": 5, "auto_refresh": false}),
        Err(e) => return Err(format!("설정 파일을 읽을 수 없어 저장하지 않았습니다: {e}")),
    };
    v[key] = val;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    write_json_atomic(&p, &v)
}

pub fn save_cookie(cookie: &str) -> Result<(), String> {
    let enc = protect(cookie.as_bytes()).ok_or("DPAPI 암호화 실패")?;
    update("opencode_cookie_dpapi", Value::String(b64_encode(&enc)))
}

/// The "start with Windows" choice is kept here too, so it can be restored if the Run key is lost.
pub fn save_autorun(on: bool) {
    let _ = update("autorun", json!(on));
}

/// Create the config file with defaults on first run so it is easy to find and edit.
pub fn ensure_exists() {
    if !path().exists() {
        let _ = update("interval_min", json!(5));
        let _ = update("auto_refresh", json!(false));
    }
}

fn protect(data: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        if CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) == 0 {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Some(v)
    }
}

fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        if CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) == 0 {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Some(v)
    }
}
