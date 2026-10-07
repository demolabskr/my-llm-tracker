//! OpenCode Go usage. There is no public API, so this uses the console's own JSON API
//! (https://opencode.ai/console/api/...) with the browser session cookie
//! `__Host-console_session`:
//!   GET /console/api/orgs                       -> [{id: "wrk_...", name}]
//!   GET /console/api/go/status  (x-org-id: id)  -> access.meters.{fiveHour, week, month}
//! Each meter has usedMicroCents / limitMicroCents (1e6 micro-cents = 1 cent) and resetsAt.

use super::{Fetched, Win};
use crate::{http, util::parse_iso};
use serde_json::Value;

const COOKIE_NAME: &str = "__Host-console_session";
const EXPIRED: &str = "세션 만료 - 트레이 메뉴에서 쿠키를 다시 붙여넣으세요";

/// Accepts the bare value ("st_..."), "__Host-console_session=st_...", or a whole
/// "Cookie: a=1; __Host-console_session=st_...; b=2" header line.
pub fn normalize_cookie(raw: &str) -> String {
    let s = raw.trim().trim_matches('"');
    let s = s.strip_prefix("Cookie:").map(str::trim).unwrap_or(s);
    if let Some(i) = s.find(COOKIE_NAME) {
        let rest = s[i + COOKIE_NAME.len()..].trim_start_matches([' ', '=']);
        return rest.split(';').next().unwrap_or("").trim().trim_matches('"').to_string();
    }
    if s.contains('=') || s.contains(';') || s.contains(' ') {
        return String::new(); // some other cookie / header, not ours
    }
    s.to_string()
}

pub fn fetch(cookie: &str) -> Fetched {
    if cookie.is_empty() {
        return Err("쿠키 미설정 - 트레이 메뉴에서 쿠키를 붙여넣으세요".into());
    }
    let cookie_hdr = format!("{COOKIE_NAME}={cookie}");
    let get = |url: &str, org: Option<&str>| {
        let mut h = vec![("Cookie", cookie_hdr.as_str()), ("Accept", "application/json")];
        if let Some(o) = org {
            h.push(("x-org-id", o));
        }
        http::get(url, &h, false)
    };

    let r = get("https://opencode.ai/console/api/orgs", None)?;
    if matches!(r.status, 301..=303 | 307 | 308 | 401 | 403) {
        return Err(EXPIRED.into());
    }
    if r.status != 200 {
        return Err(format!("HTTP {}", r.status));
    }
    let orgs = parse_orgs(&r.body)?;

    // use the first workspace that actually has a Go subscription
    let mut last = Err("Go 구독을 찾지 못함".to_string());
    for id in orgs {
        let r = get("https://opencode.ai/console/api/go/status", Some(&id))?;
        if matches!(r.status, 401 | 403) {
            return Err(EXPIRED.into());
        }
        if r.status != 200 {
            last = Err(format!("HTTP {}", r.status));
            continue;
        }
        match parse(&r.body) {
            Ok(w) => return Ok((String::new(), w)),
            Err(e) => last = Err(e),
        }
    }
    last
}

fn parse_orgs(body: &str) -> Result<Vec<String>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let ids: Vec<String> = v.as_array().map_or(vec![], |a| {
        a.iter().filter_map(|o| o["id"].as_str().map(String::from)).collect()
    });
    if ids.is_empty() {
        return Err("워크스페이스가 없음".into());
    }
    Ok(ids)
}

/// Amounts arrive as JSON strings ("1200000000"); accept plain numbers too.
fn num(v: &Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str()?.trim().parse().ok())
}

pub fn parse(body: &str) -> Result<Vec<Win>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let meters = &v["access"]["meters"];
    if !meters.is_object() {
        return Err("Go 구독 없음 (access 정보 없음)".into());
    }
    let mut out = Vec::new();
    for (key, label) in [("fiveHour", "5h"), ("week", "7d"), ("month", "30d")] {
        let m = &meters[key];
        let (Some(used), Some(limit)) = (num(&m["usedMicroCents"]), num(&m["limitMicroCents"])) else { continue };
        if limit <= 0.0 {
            continue;
        }
        let reset = m["resetsAt"].as_str().and_then(parse_iso);
        out.push(Win::from_used(label, used / limit * 100.0, reset));
    }
    if out.is_empty() {
        return Err("사용량을 찾지 못함 (API 구조 변경 가능)".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status() {
        let body = r#"{"subscriberUserId":"usr_x","useBalance":false,"access":{"startsAt":"2026-09-09T22:43:48.000Z",
            "meters":{"fiveHour":{"resetsAt":null,"limitMicroCents":1200000000,"usedMicroCents":0},
                      "week":{"startsAt":"2026-10-05T00:00:00.000Z","resetsAt":"2026-10-12T00:00:00.000Z","limitMicroCents":3000000000,"usedMicroCents":300000000},
                      "month":{"resetsAt":"2026-10-09T22:43:48.000Z","limitMicroCents":6000000000,"usedMicroCents":2254962641}}}}"#;
        let w = parse(body).unwrap();
        assert_eq!(w.len(), 3);
        assert_eq!((w[0].label.as_str(), w[0].left, w[0].reset_at), ("5h", 100.0, None));
        assert_eq!(w[1].left, 90.0);
        assert!(w[1].reset_at.is_some());
        assert!((w[2].left - 62.4).abs() < 0.1);
    }

    #[test]
    fn parses_string_amounts() {
        let body = r#"{"access":{"meters":{"fiveHour":{"startsAt":null,"resetsAt":null,"limitMicroCents":"1200000000","usedMicroCents":"0"},
            "week":{"resetsAt":"2026-10-12T00:00:00.000Z","limitMicroCents":"3000000000","usedMicroCents":"5109394"},
            "month":{"resetsAt":"2026-10-09T22:43:48.000Z","limitMicroCents":"6000000000","usedMicroCents":"2254962641"}}}}"#;
        let w = parse(body).unwrap();
        assert_eq!(w.len(), 3);
        assert_eq!(w[0].left, 100.0);
        assert!((w[1].left - 99.83).abs() < 0.01);
        assert!((w[2].left - 62.42).abs() < 0.01);
    }

    #[test]
    fn no_subscription() {
        assert!(parse(r#"{"access":null}"#).is_err());
        assert!(parse(r#"{}"#).is_err());
    }

    #[test]
    fn orgs() {
        assert_eq!(parse_orgs(r#"[{"id":"wrk_01A","name":"x"},{"id":"org_2"}]"#).unwrap(), vec!["wrk_01A", "org_2"]);
        assert!(parse_orgs("[]").is_err());
    }

    #[test]
    fn cookie_forms() {
        assert_eq!(normalize_cookie("st_example"), "st_example");
        assert_eq!(normalize_cookie("__Host-console_session=st_example"), "st_example");
        assert_eq!(normalize_cookie("Cookie: a=1; __Host-console_session=st_example; b=2"), "st_example");
        assert_eq!(normalize_cookie("a=1; b=2"), "");
    }
}
