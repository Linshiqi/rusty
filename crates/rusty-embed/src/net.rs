//! Which route a download or an index query should take.
//!
//! Its own module because three unrelated things ask: the tool installer, the
//! update check, and the Crates panel's registry query. It lived in
//! `simulate.rs`, so the answer to "where does rusty decide about proxies"
//! was a module named after the emulator — which nobody looking for it would
//! open.
//!
//! **One proxy URL is not one route.** A local proxy's HTTP CONNECT can be
//! incompatible with a client while its SOCKS listener on the same port works,
//! and a mirror may be reachable with no proxy at all. So callers get a
//! *ladder* and try it in order, naming each attempt — a download that fails
//! silently on the one machine that needs a proxy is the failure this exists
//! to prevent.

/// What downloads and index queries should actually use: the stored setting
/// first (an explicit URL, or "none" for forced direct), then detection.
pub fn effective_proxy() -> Option<String> {
    if let Some(configured) = crate::config::workbench().proxy {
        let configured = configured.trim().to_string();
        if configured.eq_ignore_ascii_case("none") || configured.is_empty() {
            return None;
        }
        if !configured.eq_ignore_ascii_case("auto") {
            return Some(configured);
        }
    }
    system_proxy()
}

/// Environment variables first (the cross-platform convention), then the
/// Windows system proxy from the registry — which is what the browser and
/// every GUI proxy tool (Clash and friends) configure. A tool that ignores
/// it downloads into a wall on exactly the machines that need a proxy.
pub fn system_proxy() -> Option<String> {
    for key in [
        "HTTPS_PROXY",
        "https_proxy",
        "HTTP_PROXY",
        "http_proxy",
        "ALL_PROXY",
    ] {
        if let Ok(value) = std::env::var(key)
            && !value.trim().is_empty()
        {
            return Some(value.trim().to_string());
        }
    }
    if cfg!(windows) {
        return windows_system_proxy();
    }
    None
}

fn windows_system_proxy() -> Option<String> {
    let query = |value: &str| -> Option<String> {
        let mut command = std::process::Command::new("reg");
        command.args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            value,
        ]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        let out = command.output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines()
            .find(|line| line.trim_start().starts_with(value))
            .and_then(|line| line.split_whitespace().last())
            .map(str::to_string)
    };

    let enabled = query("ProxyEnable")?;
    if !enabled.ends_with('1') {
        return None;
    }
    parse_proxy_server(&query("ProxyServer")?)
}

/// The registry's `ProxyServer` shapes: a bare `host:port` for everything,
/// or `http=h:p;https=h:p;ftp=…` per protocol. Https wins, then http; socks
/// entries are skipped — this client does not speak socks.
fn parse_proxy_server(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if !value.contains('=') {
        return Some(format!("http://{value}"));
    }
    let mut http = None;
    for part in value.split(';') {
        let Some((scheme, address)) = part.split_once('=') else {
            continue;
        };
        match scheme.trim() {
            "https" => return Some(format!("http://{}", address.trim())),
            "http" => http = Some(format!("http://{}", address.trim())),
            _ => {}
        }
    }
    http
}

/// Every route worth trying, in order — the configured proxy, its SOCKS
/// twin, then direct. Shared with the update check, which has the same
/// problem this module's header describes.
pub fn proxy_routes() -> Vec<Option<String>> {
    proxy_candidates(effective_proxy())
}

/// The transport ladder for one configured proxy: as given, then the SOCKS5
/// spelling of the same address (mixed-port proxies answer both), then no
/// proxy at all. Deduplicated, order kept.
///
/// Separate from [`proxy_routes`] so it can be tested without reading the
/// machine's own settings.
pub(crate) fn proxy_candidates(configured: Option<String>) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = Vec::new();
    if let Some(url) = configured {
        out.push(Some(url.clone()));
        if let Some(rest) = url.strip_prefix("http://") {
            let socks = format!("socks5://{rest}");
            if !out.contains(&Some(socks.clone())) {
                out.push(Some(socks));
            }
        }
    }
    out.push(None);
    out
}

/// Every layer of an error, because "unexpected end of file" alone points
/// nowhere — whether the proxy or the TLS or the socket said it is the
/// entire diagnosis.
pub(crate) fn error_chain(error: &dyn std::error::Error) -> String {
    let mut parts = vec![error.to_string()];
    let mut source = error.source();
    while let Some(inner) = source {
        parts.push(inner.to_string());
        source = inner.source();
    }
    parts.dedup();
    parts.join(" ← ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ladder is the whole point: one configured proxy is three attempts,
    /// and direct is always the last of them.
    #[test]
    fn a_configured_proxy_becomes_a_ladder_ending_in_direct() {
        let routes = proxy_candidates(Some("http://127.0.0.1:7890".to_string()));
        assert_eq!(
            routes,
            vec![
                Some("http://127.0.0.1:7890".to_string()),
                Some("socks5://127.0.0.1:7890".to_string()),
                None,
            ],
            "a mixed-port proxy answers both, and direct may be what works",
        );

        // An https or socks URL has no second spelling to try.
        assert_eq!(
            proxy_candidates(Some("socks5://127.0.0.1:7890".to_string())),
            vec![Some("socks5://127.0.0.1:7890".to_string()), None],
        );
        assert_eq!(
            proxy_candidates(None),
            vec![None],
            "direct is still a route"
        );
    }

    #[test]
    fn the_registry_names_a_proxy_in_two_shapes() {
        assert_eq!(
            parse_proxy_server("127.0.0.1:7890").as_deref(),
            Some("http://127.0.0.1:7890"),
            "a bare address applies to everything",
        );
        assert_eq!(
            parse_proxy_server("http=h:1;https=h:2;ftp=h:3").as_deref(),
            Some("http://h:2"),
            "https wins where both are named",
        );
        assert_eq!(
            parse_proxy_server("http=h:1;ftp=h:3").as_deref(),
            Some("http://h:1"),
            "and http stands in when it does not",
        );
        assert_eq!(
            parse_proxy_server("socks=h:9"),
            None,
            "this client does not speak socks, so naming one is not an answer",
        );
        assert_eq!(parse_proxy_server("  "), None);
    }

    #[test]
    fn an_error_chain_names_every_layer_once() {
        #[derive(Debug)]
        struct Layer(&'static str, Option<Box<Layer>>);
        impl std::fmt::Display for Layer {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        impl std::error::Error for Layer {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                self.1
                    .as_deref()
                    .map(|inner| inner as &dyn std::error::Error)
            }
        }

        let error = Layer("io error", Some(Box::new(Layer("tls closed", None))));
        assert_eq!(error_chain(&error), "io error ← tls closed");
    }
}
