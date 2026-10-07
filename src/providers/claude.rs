//! Claude (Pro/Max) usage: the same OAuth token Claude Code stores locally, against
//! the usage endpoint Claude Code itself uses for /usage. Undocumented, may change.

use super::{Fetched, Win};
use crate::{http, util::*};
use serde_json::{json, Value};
use std::path::Path;

const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const EXPIRED: &str = "토큰 만료 - 이 PC에서 claude를 한 번 실행하세요";

pub fn fetch(auto_refresh: bool) -> Fetched {
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()).map(std::path::PathBuf::from);
    let path = dir.unwrap_or_else(|| home().join(".claude")).join(".credentials.json");
    let mut cred = read_json(&path).map_err(|_| "로그인 정보 없음 (claude 로그인 필요)".to_string())?;
    let mut refreshed = false;

    if cred["claudeAiOauth"]["expiresAt"].as_i64().unwrap_or(0) - 60_000 <= now() * 1000 {
        if !auto_refresh {
            return Err(EXPIRED.into());
        }
        refresh(&mut cred, &path)?;
        refreshed = true;
    }
    let mut r = call(&cred)?;
    if r.status == 401 {
        if !auto_refresh || refreshed {
            return Err(EXPIRED.into());
        }
        refresh(&mut cred, &path)?;
        r = call(&cred)?;
    }
    if r.status != 200 {
        return Err(format!("HTTP {}", r.status));
    }
    let plan = cred["claudeAiOauth"]["subscriptionType"].as_str().unwrap_or("").to_string();
    Ok((plan, parse(&r.body)?))
}

fn call(cred: &Value) -> Result<http::Resp, String> {
    let tok = cred["claudeAiOauth"]["accessToken"].as_str().ok_or("accessToken 없음")?;
    http::get(
        "https://api.anthropic.com/api/oauth/usage",
        &[("Authorization", &format!("Bearer {tok}")), ("anthropic-beta", "oauth-2025-04-20")],
        false,
    )
}

fn refresh(cred: &mut Value, path: &Path) -> Result<(), String> {
    let rt = cred["claudeAiOauth"]["refreshToken"].as_str().ok_or("refreshToken 없음")?.to_string();
    // Claude Code sends the granted scopes with the refresh request; without them the server answers 400.
    let scope = cred["claudeAiOauth"]["scopes"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload".into());
    let body = json!({"grant_type": "refresh_token", "refresh_token": rt, "client_id": CLIENT_ID, "scope": scope}).to_string();
    let r = http::post_json("https://platform.claude.com/v1/oauth/token", &body)?;
    if r.status != 200 {
        return Err(format!("토큰 갱신 실패 (HTTP {}{})", r.status, oauth_err(&r.body)));
    }
    let v: Value = serde_json::from_str(&r.body).map_err(|e| e.to_string())?;
    let o = &mut cred["claudeAiOauth"];
    if let Some(s) = v["scope"].as_str().filter(|s| !s.is_empty()) {
        o["scopes"] = json!(s.split_whitespace().collect::<Vec<_>>());
    }
    o["accessToken"] = v["access_token"].clone();
    if v["refresh_token"].is_string() {
        o["refreshToken"] = v["refresh_token"].clone();
    }
    o["expiresAt"] = json!(now() * 1000 + v["expires_in"].as_i64().unwrap_or(3600) * 1000);
    write_json_atomic(path, cred)
}

pub fn parse(body: &str) -> Result<Vec<Win>, String> {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for (key, label) in [
        ("five_hour", "5h"),
        ("seven_day", "7d"),
        ("seven_day_opus", "7d Opus"),
        ("seven_day_sonnet", "7d Sonnet"),
    ] {
        if let Some(used) = v[key]["utilization"].as_f64() {
            let reset = v[key]["resets_at"].as_str().and_then(parse_iso);
            out.push(Win::from_used(label, used, reset));
        }
    }
    if out.is_empty() {
        return Err("사용량 정보 없음".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        let w = super::parse(
            r#"{"five_hour":{"utilization":42.0,"resets_at":"2026-10-07T12:00:00.5+00:00"},
                "seven_day":{"utilization":18,"resets_at":null},"seven_day_opus":null}"#,
        )
        .unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].left, 58.0);
        assert_eq!(w[0].reset_at, Some(1791374400));
        assert_eq!(w[1].reset_at, None);
    }
}
