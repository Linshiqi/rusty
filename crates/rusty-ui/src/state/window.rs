//! How this window was opened: the query parameters a detached editor or
//! a torn-off commit boots with.

/// The `detach` query parameter, percent-decoded — the file this window
/// exists to edit, when it is that kind of window.
pub(super) fn detached_path() -> Option<String> {
    query_param("detach")
}

/// One query parameter of this window's URL, percent-decoded. How the
/// backend tells a window what kind it is — `detach=<file>` for an editor,
/// `gitdiff=<commit>` for a torn-off commit — and `None` for the shell.
pub(super) fn query_param(name: &str) -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    let prefix = format!("{name}=");
    let raw = search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix(prefix.as_str()))?;
    // Decode %XX; the backend encodes everything outside [A-Za-z0-9._-/].
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[at + 1..at + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok().filter(|p| !p.is_empty())
}
