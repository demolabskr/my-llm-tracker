//! Config file (%APPDATA%\llm-tracker\config.json).

use crate::util::{read_json, write_json_atomic};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone)]
pub struct Config {
    pub interval_min: u64,
    pub auto_refresh: bool,
    pub autorun: bool,
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
    Config {
        interval_min: v["interval_min"].as_u64().unwrap_or(5).clamp(1, 120),
        auto_refresh: v["auto_refresh"].as_bool().unwrap_or(false),
        autorun: v["autorun"].as_bool().unwrap_or(false),
        codex_home: v["codex_home"].as_str().unwrap_or("").to_string(),
    }
}

/// Never falls back to defaults because of a transient read failure: that would silently turn
/// off auto_refresh. It keeps the last good config instead.
pub fn load() -> Config {
    let mut last = LAST_GOOD.lock().unwrap_or_else(|e| e.into_inner());
    match read_config() {
        Ok(v) => {
            if v.is_none() {
                log("load: config.json not found -> defaults");
            }
            let c = parse(&v.unwrap_or(Value::Null));
            *last = Some(c.clone());
            c
        }
        Err(e) => {
            log(&format!("load: read failed ({e}) -> {}", if last.is_some() { "last good" } else { "defaults" }));
            last.clone().unwrap_or_else(|| parse(&Value::Null))
        }
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

/// The "start with Windows" choice is kept here too, so it can be restored if the Run key is lost.
pub fn save_autorun(on: bool) {
    let _ = update("autorun", json!(on));
}

/// What this process currently sees of the config file (for debug.log).
pub fn diag() -> String {
    let p = path();
    let state = match read_json(&p) {
        Ok(_) => "read=ok".to_string(),
        Err(e) => format!("read=ERR({e})"),
    };
    format!("path={} exists={} {state}", p.display(), p.exists())
}

/// Append one line to %APPDATA%\llm-tracker\debug.log (kept under ~32 KB).
pub fn log(line: &str) {
    let lp = path().with_file_name("debug.log");
    if std::fs::metadata(&lp).map_or(false, |m| m.len() > 32 * 1024) {
        let old = std::fs::read_to_string(&lp).unwrap_or_default();
        let _ = std::fs::write(&lp, old.get(old.len() / 2..).unwrap_or(""));
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&lp) {
        let _ = writeln!(f, "{} {line}", crate::util::iso_now());
    }
}

/// Create the config file with defaults on first run so it is easy to find and edit.
pub fn ensure_exists() {
    if !path().exists() {
        let _ = update("interval_min", json!(5));
        let _ = update("auto_refresh", json!(false));
    }
}
