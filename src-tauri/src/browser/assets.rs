//! The image proxy protocol `vemry-asset://`: the reading projection's images, and a
//! note's remote images once the user lets them load.
//!
//! The projection renderer emits `![..](url)` as
//! `<img src="vemry-asset://localhost/?u=..&r=..">`, and this fetches it on its
//! behalf: **no cookies**, a Referer of the page URL (to get past hotlink
//! protection), and a UA on the scale of desktop Safari. That is how the main
//! webview keeps its "never talks to a remote host directly" property (its CSP
//! allows no remote image at all) — the egress narrows to this one Rust channel, and
//! for the projection it only happens after the user explicitly opened it. The image
//! host gets the request any browser showing that page would send, minus the
//! cookies; the reader window itself blocks images, so this is the first one.
//!
//! A note on disk comes through here only with the user's leave: opening a note must
//! stay free of network traffic — a tracking pixel in the page reached disk with its
//! `![](url)`, and if rendering a note fetched it, every open would leak "who is still
//! reading this, and when" back to the original site. So the editor asks with `c=1`,
//! the disk cache alone, and an image that is not there waits as a placeholder until
//! the user loads it (`k=note`: a click, the whole note, or the `note_remote_images`
//! setting; src/lib/editor/remoteimages.ts).
//! Clip localisation (clip.rs) reuses `fetch_image` to download once **at the moment
//! of clipping** and write into the vault — a trust level equal to the user looking
//! at the page right then, after which reading stays free of network traffic.
//!
//! SSRF protection: an image URL comes from page content and is untrusted. Only
//! http(s) is allowed; every IP the host resolves to has to be a public address
//! (private, loopback, link-local and CGN are all refused), the resolution is pinned
//! into the request against DNS rebinding, and redirects are followed by hand with
//! every hop revalidated.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tauri::http::{Request, Response};
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder};

/// A UA on the scale of desktop Safari: plenty of image hosts filter roughly by UA, and an empty one or reqwest's default gets refused.
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15";
const ACCEPT: &str = "image/avif,image/webp,image/png,image/svg+xml,image/*;q=0.8,*/*;q=0.5";
const MAX_REDIRECTS: usize = 4;
/// The size limit for one image.
const MAX_BYTES: usize = 20 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// The total limit of the disk cache; over it, the oldest are cleared by mtime down to 3/4.
const CACHE_MAX_BYTES: u64 = 256 * 1024 * 1024;

/// The protocol entry point (registered in lib.rs). An asynchronous protocol: it returns at once and the fetch happens in the runtime.
pub fn handle(ctx: UriSchemeContext<'_, tauri::Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    let uri = request.uri().to_string();
    tauri::async_runtime::spawn(async move {
        responder.respond(serve(&app, &uri).await);
    });
}

async fn serve(app: &AppHandle, uri: &str) -> Response<Vec<u8>> {
    match serve_inner(app, uri).await {
        Ok((bytes, content_type)) => Response::builder()
            .status(200)
            .header("content-type", content_type)
            // The protocol response can be cached in WKWebView's memory, so scrolling back into view does not go through the protocol again
            .header("cache-control", "public, max-age=604800")
            .body(bytes)
            .unwrap_or_else(|_| plain(500, "failed to build the response")),
        Err((status, msg)) => plain(status, &msg),
    }
}

fn plain(status: u16, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("content-type", "text/plain; charset=utf-8")
        .body(msg.as_bytes().to_vec())
        .unwrap_or_default()
}

/// What a `vemry-asset://` request asks for: `u` the image, `r` the page to name as
/// its Referer (absent or empty: none). `k=note` marks an image in a note on disk,
/// shown by the editor, which the reading projection's switch does not govern — the
/// editor asks only after its own rule allowed it (a click, or `note_remote_images`).
/// `c=1` asks the disk cache and nothing else, so an image seen before shows with no
/// network request and an unseen one answers 404 and stays a placeholder.
#[derive(Debug, PartialEq)]
struct AssetRequest {
    target: String,
    referer: Option<String>,
    note: bool,
    cache_only: bool,
}

fn parse_request(uri: &str) -> Result<AssetRequest, (u16, String)> {
    let parsed = url::Url::parse(uri).map_err(|_| (400, "could not parse the protocol URL".to_string()))?;
    let (mut target, mut referer, mut note, mut cache_only) = (None, None, false, false);
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "u" => target = Some(v.into_owned()),
            "r" if !v.is_empty() => referer = Some(v.into_owned()),
            "k" => note = v == "note",
            "c" => cache_only = v == "1",
            _ => {}
        }
    }
    let target = target.ok_or((400, "the image URL is missing".to_string()))?;
    Ok(AssetRequest { target, referer, note, cache_only })
}

async fn serve_inner(app: &AppHandle, uri: &str) -> Result<(Vec<u8>, String), (u16, String)> {
    let req = parse_request(uri)?;
    if !req.note && !crate::settings::load(app).reading_images_enabled {
        return Err((403, "loading projection images is turned off in the settings".to_string()));
    }
    if req.cache_only {
        return cache_read(app, &cache_key(&req.target)).ok_or((404, "not in the cache".to_string()));
    }
    fetch_image(app, &req.target, req.referer.as_deref()).await.map_err(|e| (502, e))
}

/// A cache-first image fetch: cache → fetch_validated → cache. Shared by the
/// protocol's serve and by clip localisation — once the projection has seen an
/// image, clipping it to disk needs no network at all.
pub(crate) async fn fetch_image(
    app: &AppHandle,
    url: &str,
    referer: Option<&str>,
) -> Result<(Vec<u8>, String), String> {
    let key = cache_key(url);
    if let Some(hit) = cache_read(app, &key) {
        return Ok(hit);
    }
    let (bytes, content_type) = fetch_validated(url, referer).await?;
    cache_write(app, &key, &bytes, &content_type);
    Ok((bytes, content_type))
}

/// The table of validated DNS results: host → pinned addresses. The shared client's
/// custom resolver knows nothing else — a host that is not in the table simply fails
/// to resolve, so the shared connection pool **structurally** cannot reach an
/// address that did not pass validate_target (DNS rebinding swapping the answer
/// between validation and connection cannot get in either: the connection only uses
/// the answer we resolved and validated).
type PinMap = Arc<RwLock<HashMap<String, Vec<SocketAddr>>>>;

struct PinnedResolver(PinMap);

impl reqwest::dns::Resolve for PinnedResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let map = Arc::clone(&self.0);
        Box::pin(async move {
            let addrs = map.read().unwrap().get(name.as_str()).cloned();
            match addrs {
                Some(a) => Ok(Box::new(a.into_iter()) as reqwest::dns::Addrs),
                None => Err(format!("unvalidated host: {}", name.as_str()).into()),
            }
        })
    }
}

/// The process-wide shared client: the connection pool is reused across requests (so
/// several images from one host no longer each redo the handshake), while the egress
/// stays locked by PinnedResolver to validated addresses.
fn shared_client() -> Result<&'static (reqwest::Client, PinMap), String> {
    static CLIENT: OnceLock<Option<(reqwest::Client, PinMap)>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            let pins: PinMap = Arc::new(RwLock::new(HashMap::new()));
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(FETCH_TIMEOUT)
                .dns_resolver(Arc::new(PinnedResolver(Arc::clone(&pins))))
                .build()
                .ok()
                .map(|c| (c, pins))
        })
        .as_ref()
        .ok_or_else(|| "the image fetch client failed to initialise".to_string())
}

/// A fetch with validation, pinning and hand-followed redirects. Every hop passes the SSRF validation again.
async fn fetch_validated(url_str: &str, referer: Option<&str>) -> Result<(Vec<u8>, String), String> {
    let mut current = url::Url::parse(url_str).map_err(|_| "invalid image URL".to_string())?;
    let (client, pins) = shared_client()?;
    for _ in 0..=MAX_REDIRECTS {
        let (host, addrs) = validate_target(&current).await?;
        {
            // Pin the validated resolution into the table: this refuses DNS rebinding
            // swapping the answer between validation and connection. Concurrent
            // requests for the same host overwriting each other is fine — every one of
            // them was validated.
            let mut m = pins.write().unwrap();
            if m.len() > 512 {
                m.clear(); // bounded: a long browsing session does not accumulate forever
            }
            m.insert(host, addrs);
        }
        let mut req = client
            .get(current.as_str())
            .header("user-agent", UA)
            .header("accept", ACCEPT);
        if let Some(r) = referer.filter(|r| r.starts_with("http://") || r.starts_with("https://")) {
            req = req.header("referer", r);
        }
        let mut resp = req.send().await.map_err(|e| format!("image fetch failed: {e}"))?;
        if resp.status().is_redirection() {
            let loc = resp
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("the redirect has no Location")?;
            current = current.join(loc).map_err(|_| "invalid redirect target".to_string())?;
            continue;
        }
        if !resp.status().is_success() {
            return Err(format!("the origin returned {}", resp.status()));
        }
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_BYTES {
                return Err("the image is too large".into());
            }
        }
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let mut buf = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
            buf.extend_from_slice(&chunk);
            if buf.len() > MAX_BYTES {
                return Err("the image is too large".into());
            }
        }
        return Ok((buf, content_type));
    }
    Err("too many redirects".into())
}

/// SSRF validation: a scheme allowlist plus every resolved address being public. Returns (host, pinned addresses).
async fn validate_target(u: &url::Url) -> Result<(String, Vec<SocketAddr>), String> {
    if !matches!(u.scheme(), "http" | "https") {
        return Err("only http(s) images are proxied".into());
    }
    let host = u.host_str().ok_or("the image URL has no host name")?.to_string();
    let port = u.port_or_known_default().unwrap_or(443);
    let addrs: Vec<SocketAddr> = match u.host() {
        Some(url::Host::Ipv4(ip)) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        Some(url::Host::Ipv6(ip)) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        _ => tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|e| format!("host lookup failed: {e}"))?
            .collect(),
    };
    if addrs.is_empty() {
        return Err("the host resolved to nothing".into());
    }
    // One non-public address is enough to refuse — an attacker often mixes one public and one internal record
    if addrs.iter().any(|a| blocked_ip(a.ip())) {
        return Err("the target address is not public; refused".into());
    }
    Ok((host, addrs))
}

/// Every non-public address is refused. An image URL comes from page content, and letting it drive a request into an internal network is SSRF.
fn blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64/10 CGN
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18/15 benchmarking range
                || (o[0] == 192 && o[1] == 0 && o[2] == 2) // TEST-NET-1
                || (o[0] == 198 && o[1] == 51 && o[2] == 100) // TEST-NET-2
                || (o[0] == 203 && o[1] == 0 && o[2] == 113) // TEST-NET-3
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return blocked_ip(IpAddr::V4(v4));
            }
            let s = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (s[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
                || (s[0] == 0x2001 && s[1] == 0xdb8) // the documentation range
        }
    }
}

// ---- the disk cache: app_cache_dir/reading-images/<sha256>.bin + .ct ----

fn cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_cache_dir().ok().map(|d| d.join("reading-images"))
}

fn cache_key(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn cache_read(app: &AppHandle, key: &str) -> Option<(Vec<u8>, String)> {
    let dir = cache_dir(app)?;
    // .ct is written second: its presence is what says the .bin is complete
    let content_type = std::fs::read_to_string(dir.join(format!("{key}.ct"))).ok()?;
    let bytes = std::fs::read(dir.join(format!("{key}.bin"))).ok()?;
    Some((bytes, content_type))
}

fn cache_write(app: &AppHandle, key: &str, bytes: &[u8], content_type: &str) {
    let Some(dir) = cache_dir(app) else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    // A unique temporary name plus rename: concurrent writes of the same URL do not tread on each other (rename replaces atomically)
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = dir.join(format!("{key}.tmp{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, dir.join(format!("{key}.bin"))).is_ok() {
        let _ = std::fs::write(dir.join(format!("{key}.ct")), content_type);
    } else {
        let _ = std::fs::remove_file(&tmp);
        return; // nothing was written, so nothing is counted
    }
    // The directory's byte count is tracked in-process: it starts from prune (the
    // only directory walk there is, which incidentally clears an over-limit left by
    // the previous run), and after that every successful write adds to it, with a
    // full directory sweep only once the limit is crossed (read_dir+stat drops from
    // once per write to once per crossing). The count is a best-effort approximation:
    // rewriting the same key counts as new (too high, so the sweep comes early),
    // while writes landing concurrently during a sweep have their additions
    // overwritten by store (too low, so the sweep comes late) — but every sweep resets
    // the count from what is really on disk, so the cache stays bounded and merely
    // hovers around the limit.
    static CACHE_SIZE: OnceLock<AtomicU64> = OnceLock::new();
    let counter = CACHE_SIZE.get_or_init(|| AtomicU64::new(prune(&dir)));
    let wrote = (bytes.len() + content_type.len()) as u64;
    if counter.fetch_add(wrote, Ordering::Relaxed) + wrote > CACHE_MAX_BYTES {
        // A single sweeper: the concurrent writes at the moment the limit is crossed
        // need not each walk the whole directory — whoever takes the flag sweeps and
        // the rest keep writing, and if it is still over the limit the next write
        // triggers it again
        static PRUNING: AtomicBool = AtomicBool::new(false);
        if PRUNING.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            counter.store(prune(&dir), Ordering::Relaxed);
            PRUNING.store(false, Ordering::Release);
        }
    }
}

/// When the cache directory is over the limit, clear the oldest by mtime down to 3/4
/// of it. The .bin and .ct of one key are cleared together.
/// Returns the directory's byte count after the sweep, for the writing side to reset
/// its counter with.
fn prune(dir: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((e.path(), meta.len(), meta.modified().ok()?))
        })
        .collect();
    let total: u64 = files.iter().map(|(_, len, _)| len).sum();
    if total <= CACHE_MAX_BYTES {
        return total;
    }
    files.sort_by_key(|(_, _, mtime)| *mtime);
    let mut remaining = total;
    for (path, len, _) in files {
        if remaining <= CACHE_MAX_BYTES / 4 * 3 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            remaining = remaining.saturating_sub(len);
        }
    }
    remaining
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn blocked_ip_rejects_private_and_special_ranges() {
        for bad in [
            "127.0.0.1",
            "10.0.0.5",
            "172.16.3.4",
            "192.168.1.1",
            "169.254.1.1",
            "0.0.0.0",
            "100.64.0.1",     // CGN
            "198.18.0.1",     // benchmark
            "192.0.2.7",      // TEST-NET
            "255.255.255.255",
            "224.0.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "fd12::9",
            "::ffff:192.168.0.1", // a v4-mapped private address
            "2001:db8::1",
        ] {
            assert!(blocked_ip(ip(bad)), "{bad} must be refused");
        }
    }

    #[test]
    fn blocked_ip_allows_public() {
        for good in ["1.1.1.1", "93.184.216.34", "2606:4700:4700::1111", "2400:3200::1"] {
            assert!(!blocked_ip(ip(good)), "{good} must be allowed");
        }
    }

    /// The editor's two kinds of request — a cache-only look and a load — and the
    /// projection's, which carries neither mark and so stays under its own switch.
    #[test]
    fn requests_parse_their_marks() {
        let base = "vemry-asset://localhost/";
        let note = parse_request(&format!("{base}?u=https%3A%2F%2Fex.com%2Fa%20b.png&k=note&c=1")).unwrap();
        assert_eq!(note, AssetRequest { target: "https://ex.com/a b.png".into(), referer: None, note: true, cache_only: true });
        let load = parse_request(&format!("{base}?u=https%3A%2F%2Fex.com%2Fa.png&r=https%3A%2F%2Fex.com%2Fpage&k=note")).unwrap();
        assert_eq!((load.note, load.cache_only, load.referer.as_deref()), (true, false, Some("https://ex.com/page")));
        let projection = parse_request(&format!("{base}?u=https%3A%2F%2Fex.com%2Fa.png&r=")).unwrap();
        assert_eq!((projection.note, projection.cache_only, projection.referer), (false, false, None), "an empty r is no Referer");
        assert_eq!(parse_request(&format!("{base}?r=x")).unwrap_err().0, 400);
    }

    #[test]
    fn cache_key_is_stable_hex() {
        let k = cache_key("https://example.com/a.png");
        assert_eq!(k.len(), 64);
        assert!(k.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(k, cache_key("https://example.com/a.png"));
        assert_ne!(k, cache_key("https://example.com/b.png"));
    }

    #[tokio::test]
    async fn validate_target_rejects_bad_schemes_and_private_ip_literals() {
        for bad in [
            "file:///etc/passwd",
            "ftp://example.com/x.png",
            "http://127.0.0.1/x.png",
            "http://[::1]/x.png",
            "http://192.168.1.10:8080/admin",
        ] {
            let u = url::Url::parse(bad).unwrap();
            assert!(validate_target(&u).await.is_err(), "{bad} must be refused");
        }
    }
}
