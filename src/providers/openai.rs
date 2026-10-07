//! OpenAI (ChatGPT / Codex) usage: the OAuth token Codex CLI stores locally, against the
//! ChatGPT backend usage endpoint Codex itself uses. Undocumented, may change.

use super::{Fetched, Win};
use crate::{http, util::*};
use serde_json::{json, Value};
use std::path::Path;

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const EXPIRED: &str = "토큰 만료 - 이 PC에서 codex를 한 번 실행하세요";

/// Codex keeps auth.json in $CODEX_HOME (default ~/.codex); config.json "codex_home" overrides both.
fn auth_path(cfg: &crate::store::Config) -> std::path::PathBuf {
    let dir = if !cfg.codex_home.is_empty() {
        cfg.codex_home.clone().into()
    } else {
        std::env::var_os("CODEX_HOME").filter(|d| !d.is_empty()).map(Into::into).unwrap_or_else(|| home().join(".codex"))
    };
    std::path::PathBuf::from(dir).join("auth.json")
}

pub fn fetch(cfg: &crate::store::Config) -> Fetched {
    let auto_refresh = cfg.auto_refresh;
    let path = auth_path(cfg);
    let mut auth = read_json(&path).map_err(|_| "로그인 정보 없음 (codex 로그인 필요)".to_string())?;
    let mut refreshed = false;

    let exp = auth["tokens"]["access_token"].as_str().and_then(jwt_exp).unwrap_or(0);
    if exp - 60 <= now() {
        if !auto_refresh {
            return Err(EXPIRED.into());
        }
        refresh(&mut auth, &path)?;
        refreshed = true;
    }
    let mut r = call(&auth)?;
    if r.status == 401 {
        if !auto_refresh || refreshed {
            return Err(EXPIRED.into());
        }
        refresh(&mut auth, &path)?;
        r = call(&auth)?;
    }
    if r.status != 200 {
        return Err(format!("HTTP {}", r.status));
    }
    parse(&r.body)
}

fn jwt_exp(tok: &str) -> Option<i64> {
    let payload = b64_decode(tok.split('.').nth(1)?)?;
    serde_json::from_slice::<Value>(&payload).ok()?["exp"].as_i64()
}

fn call(auth: &Value) -> Result<http::Resp, String> {
    let t = &auth["tokens"];
    let tok = t["access_token"].as_str().ok_or("access_token 없음")?;
    let mut h = vec![("Authorization", format!("Bearer {tok}")), ("User-Agent", "codex-cli".to_string())];
    if let Some(id) = t["account_id"].as_str() {
        h.push(("ChatGPT-Account-Id", id.to_string()));
    }
    let h: Vec<(&str, &str)> = h.iter().map(|(k, v)| (*k, v.as_str())).collect();
    http::get("https://chatgpt.com/backend-api/wham/usage", &h, false)
}

fn refresh(auth: &mut Value, path: &Path) -> Result<(), String> {
    let rt = auth["tokens"]["refresh_token"].as_str().ok_or("refresh_token 없음")?.to_string();
    let body = json!({
        "client_id": CLIENT_ID, "grant_type": "refresh_token",
        "refresh_token": rt, "scope": "openid profile email"
    })
    .to_string();
    let r = http::post_json("https://auth.openai.com/oauth/token", &body)?;
    if r.status != 200 {
        return Err(format!("토큰 갱신 실패 (HTTP {}{})", r.status, oauth_err(&r.body)));
    }
    let v: Value = serde_json::from_str(&r.body).map_err(|e| e.to_string())?;
    for k in ["id_token", "access_token", "refresh_token"] {
        if v[k].is_string() {
            auth["tokens"][k] = v[k].clone();
        }
    }
    auth["last_refresh"] = json!(iso_now());
    write_json_atomic(path, auth)
}

pub fn parse(body: &str) -> Fetched {
    let v: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for key in ["primary_window", "secondary_window"] {
        let w = &v["rate_limit"][key];
        let Some(used) = w["used_percent"].as_f64() else { continue };
        let secs = w["limit_window_seconds"].as_i64();
        let label = match secs {
            Some(18000) => "5h".to_string(),
            Some(604800) => "7d".to_string(),
            Some(s) if s % 86400 == 0 => format!("{}d", s / 86400),
            Some(s) => format!("{}h", s / 3600),
            None => key.trim_end_matches("_window").to_string(),
        };
        let reset = w["reset_at"].as_i64().or_else(|| w["reset_after_seconds"].as_i64().map(|s| now() + s));
        out.push(Win::from_used(label, used, reset));
    }
    if out.is_empty() {
        return Err("사용량 정보 없음".into());
    }
    Ok((v["plan_type"].as_str().unwrap_or("").to_string(), out))
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses() {
        let (plan, w) = super::parse(
            r#"{"plan_type":"plus","rate_limit":{"allowed":true,
              "primary_window":{"used_percent":12,"limit_window_seconds":18000,"reset_after_seconds":100,"reset_at":1791374400},
              "secondary_window":{"used_percent":40.5,"limit_window_seconds":604800,"reset_at":1791999999}}}"#,
        )
        .unwrap();
        assert_eq!(plan, "plus");
        assert_eq!((w[0].label.as_str(), w[0].left), ("5h", 88.0));
        assert_eq!(w[1].label, "7d");
    }
}
