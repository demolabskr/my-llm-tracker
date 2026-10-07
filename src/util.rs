use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(Into::into).unwrap_or_default()
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// "2026-10-07T12:34:56.789+00:00" / "...Z" -> unix seconds
pub fn parse_iso(s: &str) -> Option<i64> {
    let n = |a: usize, l: usize| -> Option<i64> { s.get(a..a + l)?.parse::<i64>().ok() };
    let (y, mo, d, h, mi, se) = (n(0, 4)?, n(5, 2)?, n(8, 2)?, n(11, 2)?, n(14, 2)?, n(17, 2)?);
    let mut t = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + se;
    let mut rest = s.get(19..)?;
    if rest.starts_with('.') {
        rest = rest.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    }
    if let Some(sign @ ('+' | '-')) = rest.chars().next() {
        let oh = rest.get(1..3)?.parse::<i64>().ok()?;
        let om = rest.get(4..6).and_then(|x| x.parse::<i64>().ok()).unwrap_or(0);
        let off = oh * 3600 + om * 60;
        t -= if sign == '+' { off } else { -off };
    }
    Some(t)
}

pub fn iso_now() -> String {
    let t = now();
    let (y, m, d) = civil_from_days(t.div_euclid(86400));
    let s = t.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

pub fn fmt_dur(secs: i64) -> String {
    if secs <= 0 {
        return "-".into();
    }
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{}m", m.max(1))
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Accepts standard and url-safe alphabets, with or without padding.
pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for ch in s.bytes() {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// ": invalid_grant" style suffix from an OAuth error body (never the tokens themselves).
pub fn oauth_err(body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let e = v["error"].as_str().or_else(|| v["error"]["type"].as_str()).unwrap_or("");
    let d = v["error_description"].as_str().or_else(|| v["error"]["message"].as_str()).unwrap_or("");
    let s = format!("{e} {d}");
    let s = s.trim();
    if s.is_empty() { String::new() } else { format!(": {}", s.chars().take(240).collect::<String>()) }
}

pub fn read_json(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| e.to_string())
}

/// Write via temp file + rename so a concurrent reader never sees a half-written file.
pub fn write_json_atomic(path: &Path, v: &Value) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso() {
        assert_eq!(parse_iso("2026-10-07T12:00:00Z"), Some(1791374400));
        assert_eq!(parse_iso("2026-10-07T12:00:00.123456+00:00"), Some(1791374400));
        assert_eq!(parse_iso("2026-10-07T21:00:00+09:00"), Some(1791374400));
        assert_eq!(iso_now().len(), 20);
    }

    #[test]
    fn b64() {
        for s in ["", "a", "ab", "abc", "abcd", "\u{1f600}한글"] {
            assert_eq!(b64_decode(&b64_encode(s.as_bytes())).unwrap(), s.as_bytes());
        }
        assert_eq!(b64_decode("eyJleHAiOjF9").unwrap(), b"{\"exp\":1}");
    }

    #[test]
    fn dur() {
        assert_eq!(fmt_dur(90_000), "1d 1h");
        assert_eq!(fmt_dur(8_000), "2h 13m");
        assert_eq!(fmt_dur(30), "1m");
    }
}
