//! Web URL normalisation shared by every place a URL is compared with another:
//! the browser's address row, a clip note's frontmatter `source:` and the
//! reverse lookup "has this citation been clipped". One rule, so what was
//! written to disk and what a citation says match after a restart.

use std::collections::HashMap;

pub use url::Url;

/// Turn user input into a URL: a bare domain gets https://, and only http(s) is
/// accepted (no file:, javascript:, tauri:). The `Url` type's `to_string` is the
/// normalised spelling (host lowercased, a trailing slash on a bare host, the
/// default port dropped).
pub fn normalize_url(input: &str) -> Result<Url, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("empty address".into());
    }
    let candidate = if s.contains("://") { s.to_string() } else { format!("https://{s}") };
    let url = Url::parse(&candidate).map_err(|e| format!("invalid address: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("unsupported scheme: {}", url.scheme()));
    }
    if url.host_str().is_none() {
        return Err("the address has no host".into());
    }
    Ok(url)
}

/// The directory prefix clip notes land in (relative to the vault root); the
/// clip crate's `CLIPS_DIR` derives its own from the same literal and a test
/// pins the two together.
pub const CLIPS_DIR: &str = "clips/";

/// Normalised form → the caller's spelling, for a batch of URLs (an unparsable
/// one is skipped: it can never match a clip's `source:`).
pub fn normalized_index(urls: &[String]) -> HashMap<String, &str> {
    urls.iter().filter_map(|s| normalize_url(s).ok().map(|u| (u.to_string(), s.as_str()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_bare_domain_and_rejects_other_schemes() {
        assert_eq!(normalize_url("Example.com").unwrap().to_string(), "https://example.com/");
        assert_eq!(normalize_url(" https://a.b/c?x=1 ").unwrap().to_string(), "https://a.b/c?x=1");
        assert!(normalize_url("file:///etc/passwd").is_err());
        assert!(normalize_url("javascript:alert(1)").is_err());
        assert!(normalize_url("").is_err());
    }

    #[test]
    fn normalized_index_keys_by_normal_form() {
        let urls = vec!["EXAMPLE.com/a".to_string(), "nonsense://".to_string()];
        let idx = normalized_index(&urls);
        assert_eq!(idx.get("https://example.com/a").copied(), Some("EXAMPLE.com/a"));
        assert_eq!(idx.len(), 1);
    }
}
