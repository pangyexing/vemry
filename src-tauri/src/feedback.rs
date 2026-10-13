//! Feedback: the client's side of `POST /v1/feedback` — what the feedback dialog
//! sends, and the diagnostics it shows before sending.
//!
//! Network exit 6 of docs/ARCHITECTURE.md, and the one request on it that needs
//! no sign-in. It goes out only when the user presses Send in the dialog, and
//! carries the kind, the text they wrote, a reply address — the account's when
//! signed in (the device token rides along and the server reads the address
//! from it), the one they typed when not, or none — and, while the box is ticked,
//! the four lines of `Diagnostics` exactly as the dialog showed them. Nothing
//! from a vault is attached: no note, no path, no log.

use std::time::Duration;

use serde::Serialize;
use serde_json::json;
use tauri::AppHandle;

use crate::account;
use crate::i18n::{self, t};

const TIMEOUT: Duration = Duration::from_secs(20);
/// Where "send it by e-mail instead" writes to: the support address the README and
/// the site give.
const SUPPORT_ADDRESS: &str = "support@vemry.app";
/// How much of the text that mail link carries: some mail handlers refuse a link
/// much past two thousand characters.
const MAIL_TEXT_CHARS: usize = 1500;

/// What the dialog lists under "Include diagnostics", and what is sent while it
/// is ticked: the same function answers both, so the two cannot differ. English
/// data rather than copy — a version, a platform, a language tag and a model id
/// read the same to the user and to the developer. Mirrored as
/// `FeedbackDiagnostics` in api.ts.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Diagnostics {
    pub app_version: String,
    pub os: String,
    pub lang: String,
    pub model: String,
}

pub fn diagnostics(app: &AppHandle) -> Diagnostics {
    Diagnostics {
        app_version: app.package_info().version.to_string(),
        os: os_line(),
        lang: i18n::current().tag().to_string(),
        model: crate::settings::load(app).chat_model_line(),
    }
}

/// `macOS 27.0 (aarch64)`, `Windows 10.0.26100.2033 (x86_64)`, `Ubuntu 24.04.1 LTS (x86_64)`:
/// the system and its version where one can be read without a new dependency, else
/// the platform alone.
fn os_line() -> String {
    let arch = std::env::consts::ARCH;
    let name = match std::env::consts::OS {
        "macos" => Some(format!("macOS {}", command_line("sw_vers", &["-productVersion"]).unwrap_or_default())),
        "windows" => Some(format!("Windows {}", windows_version().unwrap_or_default())),
        _ => linux_pretty_name(),
    };
    let name = name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).unwrap_or_else(|| std::env::consts::OS.to_string());
    format!("{name} ({arch})")
}

/// A command's first line of output, when it ran and printed one.
fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok().filter(|o| o.status.success())?;
    let s = String::from_utf8_lossy(&out.stdout).lines().next()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// `ver`'s bracketed version (`Microsoft Windows [Version 10.0.26100.2033]`, the
/// word before the number in the system's language), run without a console window.
#[cfg(windows)]
fn windows_version() -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("cmd").args(["/C", "ver"]).creation_flags(CREATE_NO_WINDOW).output().ok()?;
    bracketed_version(&String::from_utf8_lossy(&out.stdout))
}

#[cfg(not(windows))]
fn windows_version() -> Option<String> {
    None
}

/// The last word inside the brackets of `ver`'s answer.
#[cfg_attr(not(windows), allow(dead_code))]
fn bracketed_version(ver: &str) -> Option<String> {
    let inside = ver.split('[').nth(1)?.split(']').next()?;
    inside.split_whitespace().last().filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit())).map(str::to_owned)
}

/// `PRETTY_NAME` of /etc/os-release, unquoted.
fn linux_pretty_name() -> Option<String> {
    let text = std::fs::read_to_string("/etc/os-release").ok()?;
    pretty_name(&text)
}

fn pretty_name(os_release: &str) -> Option<String> {
    let v = os_release.lines().find_map(|l| l.strip_prefix("PRETTY_NAME="))?;
    let v = v.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// Send one feedback. `email` is read only while signed out; `include` attaches
/// the diagnostics. Errors are in the user's words; a 401 also clears a sign-in the
/// server no longer knows, as every other account call does.
pub async fn send(app: &AppHandle, kind: &str, text: &str, email: Option<&str>, include: bool) -> Result<(), String> {
    let lang = i18n::current();
    let text = text.trim();
    if text.is_empty() {
        return Err(t(lang, "feedback.error.empty"));
    }
    let token = account::cached(app).token;
    let mut body = json!({ "kind": kind, "text": text });
    if token.is_empty() {
        if let Some(e) = email.map(str::trim).filter(|e| !e.is_empty()) {
            body["email"] = json!(e);
        }
    }
    if include {
        body["diagnostics"] = json!(diagnostics(app));
    }
    let mut req = vemry_agent::client::shared_client()
        .post(format!("{}/v1/feedback", vemry_agent::provider::hosted_base_url()))
        .header(reqwest::header::USER_AGENT, vemry_agent::client::user_agent())
        .timeout(TIMEOUT)
        .json(&body);
    if !token.is_empty() {
        req = req.bearer_auth(&token);
    }
    let resp = req.send().await.map_err(|e| i18n::tf(lang, "feedback.error.network", &[("error", &e.to_string())]))?;
    let status = resp.status();
    if status.is_success() {
        return Ok(());
    }
    let body = resp.text().await.unwrap_or_default();
    let code = account::error_code(&body);
    Err(match (status.as_u16(), code.as_deref()) {
        (401, _) | (_, Some("auth_invalid")) => {
            account::on_unauthorized(app);
            t(lang, "ipc.hosted.auth_invalid")
        }
        (_, Some("email_invalid")) => t(lang, "ipc.hosted.email_invalid"),
        (429, _) => t(lang, "feedback.error.rateLimited"),
        (s, _) => i18n::tf(lang, "feedback.error.failed", &[("error", &format!("HTTP {s}: {}", body.chars().take(300).collect::<String>()))]),
    })
}

/// The fallback when the server cannot be reached: the same feedback as a message
/// in the user's own mail program, to support, with the kind and — when ticked —
/// the diagnostics below the text. Nothing leaves until they send it from there.
fn mail_link(subject: &str, kind: &str, text: &str, diagnostics: Option<&Diagnostics>) -> String {
    use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
    // RFC 6068: everything but the unreserved characters is escaped, and a line break is CRLF
    const SET: &percent_encoding::AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');
    let mut body: String = text.trim().chars().take(MAIL_TEXT_CHARS).collect();
    body.push_str(&format!("\n\n-- \nkind: {kind}\n"));
    if let Some(d) = diagnostics {
        body.push_str(&format!("vemry {}\n{}\n{}\n{}\n", d.app_version, d.os, d.lang, d.model));
    }
    let enc = |s: &str| utf8_percent_encode(&s.replace("\r\n", "\n").replace('\n', "\r\n"), SET).to_string();
    format!("mailto:{SUPPORT_ADDRESS}?subject={}&body={}", enc(subject), enc(&body))
}

// ---- IPC ----

#[tauri::command]
pub fn feedback_diagnostics(app: AppHandle) -> Diagnostics {
    diagnostics(&app)
}

/// Open the feedback as a mail to support in the system's mail program (`mail_link`).
#[tauri::command]
pub fn feedback_mail_open(app: AppHandle, kind: String, text: String, diagnostics: bool) -> Result<(), String> {
    let d = diagnostics.then(|| self::diagnostics(&app));
    let link = mail_link(&t(i18n::current(), "feedback.mailSubject"), &kind, &text, d.as_ref());
    opener::open(link).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn feedback_send(app: AppHandle, kind: String, text: String, email: Option<String>, diagnostics: bool) -> Result<(), String> {
    send(&app, &kind, &text, email.as_deref(), diagnostics).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ver_answers_in_any_language_give_the_number() {
        assert_eq!(bracketed_version("\r\nMicrosoft Windows [Version 10.0.26100.2033]\r\n").as_deref(), Some("10.0.26100.2033"));
        assert_eq!(bracketed_version("Microsoft Windows [版本 10.0.22631.4317]").as_deref(), Some("10.0.22631.4317"));
        assert_eq!(bracketed_version("no brackets here"), None);
        assert_eq!(bracketed_version("[Version]"), None);
    }

    #[test]
    fn os_release_pretty_name_is_unquoted() {
        let text = "NAME=\"Ubuntu\"\nPRETTY_NAME=\"Ubuntu 24.04.1 LTS\"\nID=ubuntu\n";
        assert_eq!(pretty_name(text).as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(pretty_name("PRETTY_NAME=Arch Linux\n").as_deref(), Some("Arch Linux"));
        assert_eq!(pretty_name("PRETTY_NAME=\"\"\n"), None);
        assert_eq!(pretty_name("ID=void\n"), None);
    }

    #[test]
    fn the_mail_link_escapes_everything_and_breaks_lines_as_crlf() {
        let d = Diagnostics { app_version: "1.0.2".into(), os: "macOS 27.0 (aarch64)".into(), lang: "zh-CN".into(), model: "none".into() };
        let link = mail_link("vemry 反馈", "bug", " a&b=c\nnext ", Some(&d));
        assert!(link.starts_with("mailto:support@vemry.app?subject=vemry%20%E5%8F%8D%E9%A6%88&body=a%26b%3Dc%0D%0Anext%0D%0A%0D%0A--%20%0D%0Akind%3A%20bug%0D%0Avemry%201.0.2%0D%0A"), "{link}");
        assert!(!link.contains(' ') && !link.contains('\n'), "{link}");
        // Without the diagnostics, the kind alone follows the text; a long text is cut
        let link = mail_link("s", "idea", &"x".repeat(MAIL_TEXT_CHARS + 50), None);
        assert!(link.ends_with(&format!("{}%0D%0A%0D%0A--%20%0D%0Akind%3A%20idea%0D%0A", "x".repeat(10))), "{link}");
        assert_eq!(link.matches('x').count(), MAIL_TEXT_CHARS);
    }

    #[test]
    fn this_machine_has_an_os_line() {
        let line = os_line();
        assert!(line.ends_with(&format!("({})", std::env::consts::ARCH)), "{line}");
        assert!(!line.starts_with('('), "{line}");
    }
}
