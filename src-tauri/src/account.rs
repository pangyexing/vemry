//! The hosted account: the client's side of the account protocol (sign-in by
//! e-mail code, the device token, the entitlement snapshot, redeeming a code)
//! and the one file it lives in.
//!
//! `account.json` sits in the OS app-config directory beside settings.json but
//! is a separate file with mode 0600: exporting or copying the settings never
//! carries the token along. The token is the account's device credential and,
//! in hosted mode, the relay's API key — settings.rs builds the hosted
//! `ProviderConfig` from the in-process copy kept here (`cached`), so the
//! request path never reads the disk.
//!
//! Freshness is the offline rule: a snapshot fetched within ten minutes is
//! `fresh`; within seven days it is `stale` and still applies (a machine that
//! cannot reach the service keeps its Pro); past seven days it is `expired` and
//! the account counts as free and not hosted until a refresh succeeds. A 401 from
//! anywhere — a refresh, a relay call — means the token was revoked: the file is
//! cleared and the frontend is told to ask for a sign-in.
//!
//! Network exit 6 of docs/ARCHITECTURE.md: nothing here runs unless the user
//! signs in or turns hosted mode on. What leaves the machine is the e-mail
//! address at sign-in and, from then on, the requests the relay forwards —
//! byte for byte what bring-your-own-key mode would send to the vendor.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::RwLock;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use vemry_agent::provider::{hosted_base_url, ModelCap};

use crate::config_store;
use crate::i18n::{self, t};

const FILE: &str = "account.json";
/// The event every change of account state goes out on (the payload is an
/// `AccountStatus`); the frontend's account store listens for it.
pub const EVENT: &str = "account://changed";
const FRESH_SECS: i64 = 10 * 60;
const GRACE_SECS: i64 = 7 * 24 * 60 * 60;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// What account.json holds. `me` and `etag` are what the last successful
/// `/v1/me` (or the sign-in, or a redeem) returned; `fetched_at` is when.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountFile {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub me: Option<Me>,
    #[serde(default)]
    pub fetched_at: Option<String>,
    #[serde(default)]
    pub etag: Option<String>,
}

/// The entitlement snapshot (`GET /v1/me`). Every field defaults, so a server
/// that adds one — or an older snapshot on disk — still parses; the frontend's
/// `Me` in api.ts mirrors this shape.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Me {
    #[serde(default)]
    pub email: String,
    /// `free` | `pro` | `founder`.
    #[serde(default)]
    pub plan: String,
    #[serde(default)]
    pub pro_until: Option<String>,
    /// Hosted model access is open to this account (decided by what was bought).
    #[serde(default)]
    pub hosted: bool,
    #[serde(default)]
    pub quota: Option<Quota>,
    /// The credit pack, in fen; negative while an overdraft awaits the next credit.
    #[serde(default)]
    pub pack_fen: i64,
    #[serde(default)]
    pub subscription: Option<Subscription>,
    #[serde(default)]
    pub estimates: Estimates,
    #[serde(default)]
    pub models: Vec<ModelCap>,
    #[serde(default)]
    pub workflow_free_uses: u32,
    #[serde(default)]
    pub portal_url: String,
    #[serde(default)]
    pub server_time: String,
    #[serde(default)]
    pub peak: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Quota {
    #[serde(default)]
    pub total_fen: i64,
    #[serde(default)]
    pub used_fen: i64,
    #[serde(default)]
    pub resets_at: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Subscription {
    /// `active` | `canceling` | `canceled` | `past_due`.
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub cancel_at_period_end: bool,
    #[serde(default)]
    pub next_charge_at: Option<String>,
    #[serde(default)]
    pub price: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Estimates {
    /// What one study run costs on average, in fen: "about N papers left" divides by it.
    #[serde(default)]
    pub study_fen: i64,
    #[serde(default)]
    pub translate_fen: i64,
}

/// How old the snapshot is, and what that means (see the module comment).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    Fresh,
    Stale,
    Expired,
}

/// What the frontend sees (mirrored as `AccountStatus` in api.ts): the snapshot,
/// how old it is, and the two derived answers every surface needs.
#[derive(Clone, Debug, Serialize)]
pub struct AccountStatus {
    pub logged_in: bool,
    pub me: Option<Me>,
    pub fetched_at: Option<String>,
    pub freshness: Freshness,
    /// Hosted requests may go out: signed in, the account has hosted access, and
    /// the snapshot is not expired.
    pub hosted_ok: bool,
    /// `floor(quota remaining / study_fen)`, or None without a quota or an estimate.
    pub study_left: Option<i64>,
}

// ---- the file and its in-process copy ----

/// The in-process copy: `provider_config` builds the hosted config from it on
/// every request, and the file is read once at startup (`warm`) and after every
/// write here. Editing account.json by hand needs a restart, like settings.json.
static CACHE: RwLock<Option<AccountFile>> = RwLock::new(None);

/// Read the file into the cache (startup).
pub fn warm(app: &AppHandle) {
    let f: AccountFile = config_store::read(app, FILE);
    *CACHE.write().unwrap() = Some(f);
}

/// The current account file (the cache; the disk when the cache is cold).
pub fn cached(app: &AppHandle) -> AccountFile {
    if let Some(f) = CACHE.read().unwrap().as_ref() {
        return f.clone();
    }
    let f: AccountFile = config_store::read(app, FILE);
    *CACHE.write().unwrap() = Some(f.clone());
    f
}

/// The token and the catalog for the hosted provider, when signed in. Read from
/// the in-process copy alone (`Settings::provider_config` has no app handle;
/// `warm` fills the copy at startup, and a cold one means signed out). Nothing
/// about freshness here: run_agent_turn checks `status` first, and a stale
/// snapshot's catalog is the right one to build a request with.
pub fn hosted_credentials() -> Option<(String, Vec<ModelCap>)> {
    let f = CACHE.read().unwrap().clone()?;
    if f.token.is_empty() {
        return None;
    }
    Some((f.token, f.me.map(|m| m.models).unwrap_or_default()))
}

fn save(app: &AppHandle, f: &AccountFile) -> Result<(), String> {
    config_store::save_private(app, FILE, f)?;
    *CACHE.write().unwrap() = Some(f.clone());
    Ok(())
}

/// Forget the token and the snapshot (sign-out, or a revoked token).
fn clear(app: &AppHandle) {
    if let Some(p) = config_store::path(app, FILE) {
        let _ = std::fs::remove_file(p);
    }
    *CACHE.write().unwrap() = Some(AccountFile::default());
}

// ---- status ----

pub fn status(app: &AppHandle) -> AccountStatus {
    status_of(&cached(app), Utc::now())
}

/// The status of a file at a given moment — the pure half, so the rules have tests.
fn status_of(f: &AccountFile, now: DateTime<Utc>) -> AccountStatus {
    let logged_in = !f.token.is_empty();
    let freshness = freshness_at(f.fetched_at.as_deref(), now);
    let me = if logged_in { f.me.clone() } else { None };
    let hosted_ok = logged_in && freshness != Freshness::Expired && me.as_ref().is_some_and(|m| m.hosted);
    let study_left = me.as_ref().and_then(study_left);
    AccountStatus { logged_in, me, fetched_at: f.fetched_at.clone(), freshness, hosted_ok, study_left }
}

fn freshness_at(fetched_at: Option<&str>, now: DateTime<Utc>) -> Freshness {
    let Some(at) = fetched_at.and_then(|s| DateTime::parse_from_rfc3339(s).ok()) else {
        return Freshness::Expired;
    };
    let age = now.signed_duration_since(at.with_timezone(&Utc)).num_seconds();
    if age <= FRESH_SECS {
        Freshness::Fresh
    } else if age <= GRACE_SECS {
        Freshness::Stale
    } else {
        Freshness::Expired
    }
}

/// "About N papers left this month": the quota's remainder over the study
/// estimate. The credit pack is not "this month" and stays out of it.
fn study_left(me: &Me) -> Option<i64> {
    let q = me.quota.as_ref()?;
    let per = me.estimates.study_fen;
    if per <= 0 {
        return None;
    }
    Some((q.total_fen - q.used_fen).max(0) / per)
}

fn emit(app: &AppHandle, st: &AccountStatus) {
    let _ = app.emit(EVENT, st);
}

// ---- the relay's error contract ----

/// The `code` of a relay error body (`{"error":{"code":…}}`) found anywhere in
/// `text` — the transport wraps the body as `HTTP <status> <reason>: <body>`, and
/// the runner wraps that again, so the JSON is located by its first brace rather
/// than by position.
fn error_code(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let v: Value = serde_json::from_str(text[start..].trim()).ok()?;
    v.pointer("/error/code").and_then(|c| c.as_str()).map(str::to_owned)
}

/// The HTTP status the transport wrote into the message (`HTTP 402 …`).
fn http_status(text: &str) -> Option<u16> {
    let rest = &text[text.find("HTTP ")? + 5..];
    rest.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// The catalog key for a relay error: by its code when the catalog knows it,
/// else by the status's default code. None when neither maps (not a relay
/// error, or one with no wording of its own).
fn error_key(code: Option<&str>, status: Option<u16>) -> Option<String> {
    let by_code = code.map(|c| format!("ipc.hosted.{c}")).filter(|k| i18n::has_key(k));
    by_code.or_else(|| {
        let fallback = match status? {
            401 => "auth_invalid",
            402 => "insufficient_balance",
            403 => "hosted_not_allowed",
            429 => "rate_limited",
            502 => "upstream_unavailable",
            503 => "upstream_busy",
            _ => return None,
        };
        Some(format!("ipc.hosted.{fallback}"))
    })
}

/// A relay error, in the user's words. `text` is any error string that may wrap
/// a relay response (an `AgentError`'s Display, a translate job's failure); the
/// result is None when it is not one. A 401 is acted on here too: the token is
/// gone on the server, so the file is cleared and the frontend told — every
/// caller wants that, and there is no second place a 401 could be noticed.
pub fn localize_relay_error(app: &AppHandle, text: &str) -> Option<String> {
    let status = http_status(text);
    let code = error_code(text);
    if status == Some(401) || code.as_deref() == Some("auth_invalid") {
        on_unauthorized(app);
    }
    error_key(code.as_deref(), status).map(|k| t(i18n::current(), &k))
}

/// The error body of a non-2xx account call, in the user's words (the same table
/// as the relay's, with the sign-in codes), else the raw text — a message the
/// user can report is better than a blank.
fn account_error(app: &AppHandle, status: u16, body: &str) -> String {
    let code = error_code(body);
    if status == 401 || code.as_deref() == Some("auth_invalid") {
        on_unauthorized(app);
    }
    match error_key(code.as_deref(), Some(status)) {
        Some(k) => t(i18n::current(), &k),
        None => i18n::tf(i18n::current(), "ipc.hosted.failed", &[("error", &format!("HTTP {status}: {}", body.chars().take(300).collect::<String>()))]),
    }
}

fn on_unauthorized(app: &AppHandle) {
    if cached(app).token.is_empty() {
        return;
    }
    clear(app);
    emit(app, &status(app));
}

// ---- the calls ----

fn base() -> String {
    hosted_base_url()
}

fn client() -> &'static reqwest::Client {
    vemry_agent::client::shared_client()
}

async fn send(app: &AppHandle, req: reqwest::RequestBuilder) -> Result<(reqwest::StatusCode, reqwest::header::HeaderMap, String), String> {
    let resp = req
        .header(reqwest::header::USER_AGENT, vemry_agent::client::user_agent())
        .timeout(REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(|e| i18n::tf(i18n::current(), "ipc.hosted.network", &[("error", &e.to_string())]))?;
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() && status != reqwest::StatusCode::NOT_MODIFIED {
        return Err(account_error(app, status.as_u16(), &body));
    }
    Ok((status, headers, body))
}

/// The device this sign-in names in the account's device list.
fn device_info(app: &AppHandle) -> Value {
    let platform = match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    };
    json!({ "name": device_name(), "platform": platform, "app_version": app.package_info().version.to_string() })
}

/// The machine's name: the environment's, else the `hostname` command's, else
/// the platform — a label the user recognises in the device list, nothing more.
fn device_name() -> String {
    let from_env = ["HOSTNAME", "COMPUTERNAME"].iter().filter_map(|k| std::env::var(k).ok()).find(|v| !v.trim().is_empty());
    from_env
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .map(|s| s.trim_end_matches(".local").to_string())
        .unwrap_or_else(|| format!("{} {}", std::env::consts::OS, std::env::consts::ARCH))
}

/// Ask for a sign-in code. The server answers 200 whatever the address — only a
/// malformed one or a rate limit is an error.
pub async fn login_start(app: &AppHandle, email: &str) -> Result<(), String> {
    let email = email.trim();
    if email.is_empty() || !email.contains('@') {
        return Err(t(i18n::current(), "ipc.hosted.email_invalid"));
    }
    send(app, client().post(format!("{}/v1/auth/otp", base())).json(&json!({ "email": email }))).await?;
    Ok(())
}

/// Turn the code into a device token, and the account into the current one.
pub async fn login_verify(app: &AppHandle, email: &str, code: &str) -> Result<AccountStatus, String> {
    let body = json!({ "email": email.trim(), "code": code.trim(), "device": device_info(app) });
    let (_, headers, text) = send(app, client().post(format!("{}/v1/auth/verify", base())).json(&body)).await?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let token = v.get("token").and_then(|t| t.as_str()).filter(|t| !t.is_empty()).ok_or("no token in the answer")?.to_string();
    let me: Me = serde_json::from_value(v.get("me").cloned().unwrap_or(Value::Null)).unwrap_or_default();
    let f = AccountFile { token, me: Some(me), fetched_at: Some(Utc::now().to_rfc3339()), etag: etag_of(&headers) };
    save(app, &f)?;
    let st = status(app);
    emit(app, &st);
    Ok(st)
}

/// Sign out: the server revokes this device (best effort — the file is cleared
/// either way, and a token the server still knows is a dead one it will refuse).
pub async fn logout(app: &AppHandle) {
    let f = cached(app);
    if !f.token.is_empty() {
        let _ = client()
            .post(format!("{}/v1/auth/logout", base()))
            .bearer_auth(&f.token)
            .header(reqwest::header::USER_AGENT, vemry_agent::client::user_agent())
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await;
    }
    clear(app);
    emit(app, &status(app));
}

/// Refresh the snapshot (`GET /v1/me`, with the ETag so an unchanged one costs a
/// 304). A network failure keeps the snapshot as it is and reports; a 401 clears
/// it (see `on_unauthorized`).
pub async fn refresh(app: &AppHandle) -> Result<AccountStatus, String> {
    let f = cached(app);
    if f.token.is_empty() {
        return Err(t(i18n::current(), "ipc.hosted.signIn"));
    }
    let mut req = client().get(format!("{}/v1/me", base())).bearer_auth(&f.token);
    if let Some(e) = &f.etag {
        req = req.header(reqwest::header::IF_NONE_MATCH, e);
    }
    let (status_code, headers, text) = send(app, req).await?;
    let mut next = f;
    if status_code == reqwest::StatusCode::NOT_MODIFIED {
        next.fetched_at = Some(Utc::now().to_rfc3339());
    } else {
        let me: Me = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        next.me = Some(me);
        next.fetched_at = Some(Utc::now().to_rfc3339());
        next.etag = etag_of(&headers);
    }
    save(app, &next)?;
    let st = status(app);
    emit(app, &st);
    Ok(st)
}

/// Redeem an invite or a purchase code; the answer carries the fresh snapshot.
pub async fn redeem(app: &AppHandle, code: &str) -> Result<AccountStatus, String> {
    let f = cached(app);
    if f.token.is_empty() {
        return Err(t(i18n::current(), "ipc.hosted.signIn"));
    }
    let code = code.trim();
    if code.is_empty() {
        return Err(t(i18n::current(), "ipc.hosted.code_invalid"));
    }
    let (_, headers, text) =
        send(app, client().post(format!("{}/v1/redeem", base())).bearer_auth(&f.token).json(&json!({ "code": code }))).await?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let me: Me = serde_json::from_value(v.get("me").cloned().unwrap_or(Value::Null)).map_err(|e| e.to_string())?;
    let next = AccountFile { me: Some(me), fetched_at: Some(Utc::now().to_rfc3339()), etag: etag_of(&headers), ..f };
    save(app, &next)?;
    let st = status(app);
    emit(app, &st);
    Ok(st)
}

fn etag_of(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers.get(reqwest::header::ETAG).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

// ---- IPC ----

#[tauri::command]
pub fn account_status(app: AppHandle) -> AccountStatus {
    status(&app)
}

#[tauri::command]
pub async fn account_login_start(app: AppHandle, email: String) -> Result<(), String> {
    login_start(&app, &email).await
}

#[tauri::command]
pub async fn account_login_verify(app: AppHandle, email: String, code: String) -> Result<AccountStatus, String> {
    login_verify(&app, &email, &code).await
}

#[tauri::command]
pub async fn account_logout(app: AppHandle) {
    logout(&app).await
}

#[tauri::command]
pub async fn account_refresh(app: AppHandle) -> Result<AccountStatus, String> {
    refresh(&app).await
}

#[tauri::command]
pub async fn account_redeem(app: AppHandle, code: String) -> Result<AccountStatus, String> {
    redeem(&app, &code).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn me(hosted: bool) -> Me {
        Me {
            email: "a@example.com".into(),
            plan: "pro".into(),
            hosted,
            quota: Some(Quota { total_fen: 1000, used_fen: 370, resets_at: None }),
            pack_fen: 3000,
            estimates: Estimates { study_fen: 63, translate_fen: 9 },
            ..Default::default()
        }
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    /// Ten minutes is fresh, seven days is the grace, past that the snapshot no
    /// longer counts; no timestamp at all is expired.
    #[test]
    fn freshness_follows_the_two_limits() {
        let now = Utc.with_ymd_and_hms(2026, 10, 13, 12, 0, 0).unwrap();
        assert_eq!(freshness_at(Some("2026-10-13T11:55:00Z"), now), Freshness::Fresh);
        assert_eq!(freshness_at(Some("2026-10-13T11:00:00Z"), now), Freshness::Stale);
        assert_eq!(freshness_at(Some("2026-10-07T12:00:00Z"), now), Freshness::Stale); // exactly seven days
        assert_eq!(freshness_at(Some("2026-10-06T11:59:00Z"), now), Freshness::Expired);
        assert_eq!(freshness_at(None, now), Freshness::Expired);
        assert_eq!(freshness_at(Some("not a date"), now), Freshness::Expired);
    }

    /// hosted_ok needs all three: a token, the flag, and a snapshot within the grace;
    /// study_left is the quota's remainder over the estimate.
    #[test]
    fn status_derives_hosted_ok_and_study_left() {
        let now = at("2026-10-13T12:00:00Z");
        let f = AccountFile { token: "t".into(), me: Some(me(true)), fetched_at: Some("2026-10-13T11:59:00Z".into()), etag: None };
        let st = status_of(&f, now);
        assert!(st.logged_in && st.hosted_ok && st.freshness == Freshness::Fresh);
        assert_eq!(st.study_left, Some(10)); // (1000 − 370) / 63
        // Stale still applies
        let stale = AccountFile { fetched_at: Some("2026-10-10T12:00:00Z".into()), ..f.clone() };
        assert!(status_of(&stale, now).hosted_ok);
        // Expired does not
        let expired = AccountFile { fetched_at: Some("2026-09-01T12:00:00Z".into()), ..f.clone() };
        let st = status_of(&expired, now);
        assert!(st.logged_in && !st.hosted_ok && st.freshness == Freshness::Expired);
        // No hosted product
        assert!(!status_of(&AccountFile { me: Some(me(false)), ..f.clone() }, now).hosted_ok);
        // Signed out: no snapshot is shown either
        let out = status_of(&AccountFile { token: String::new(), ..f }, now);
        assert!(!out.logged_in && !out.hosted_ok && out.me.is_none() && out.study_left.is_none());
    }

    /// No quota (free, or a founder without one) or no estimate: no number rather than a division by zero.
    #[test]
    fn study_left_needs_a_quota_and_an_estimate() {
        assert_eq!(study_left(&Me { quota: None, ..me(true) }), None);
        assert_eq!(study_left(&Me { estimates: Estimates::default(), ..me(true) }), None);
        let over = Me { quota: Some(Quota { total_fen: 100, used_fen: 250, resets_at: None }), ..me(true) };
        assert_eq!(study_left(&over), Some(0));
    }

    /// The relay's error body is found through the transport's and the runner's
    /// wrapping, and the status beside it.
    #[test]
    fn relay_error_code_and_status_are_read_through_the_wrapping() {
        let text = r#"api error: HTTP 402 Payment Required: {"error":{"code":"insufficient_balance","message":"…","request_id":"r1"}}"#;
        assert_eq!(error_code(text).as_deref(), Some("insufficient_balance"));
        assert_eq!(http_status(text), Some(402));
        assert_eq!(error_key(error_code(text).as_deref(), http_status(text)).as_deref(), Some("ipc.hosted.insufficient_balance"));
        // A body the catalog has no wording for falls back to the status's default
        let odd = r#"HTTP 429 Too Many Requests: {"error":{"code":"something_new"}}"#;
        assert_eq!(error_key(error_code(odd).as_deref(), http_status(odd)).as_deref(), Some("ipc.hosted.rate_limited"));
        // Not a relay error: nothing
        assert_eq!(error_key(None, Some(500)), None);
        assert_eq!(error_key(error_code("stream idle timeout").as_deref(), http_status("stream idle timeout")), None);
        // A vendor's own 4xx body (bring-your-own-key) has no error.code and a status this table does not map
        assert_eq!(error_key(error_code(r#"HTTP 400 Bad Request: {"error":{"message":"x"}}"#).as_deref(), Some(400)), None);
    }

    /// A snapshot from an older client, or one the server has grown: unknown fields
    /// are ignored and missing ones default.
    #[test]
    fn me_parses_leniently() {
        let m: Me = serde_json::from_str(r#"{"email":"a@b.c","plan":"pro","hosted":true,"new_field":1,"models":[{"id":"x","default":true}]}"#).unwrap();
        assert!(m.hosted && m.quota.is_none() && m.models[0].default && m.models[0].max_tokens.is_none());
        let f: AccountFile = serde_json::from_str("{}").unwrap();
        assert!(f.token.is_empty() && f.me.is_none());
    }
}
