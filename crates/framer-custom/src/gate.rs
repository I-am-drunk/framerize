//! Outbound-URL gate: defense-in-depth against SSRF through the custom route.
//!
//! Policy: only absolute http/https URLs to public hosts. Everything else --
//! wrong scheme, userinfo, loopback, link-local, private ranges, unique-local
//! v6, over-long URLs, control bytes -- fails closed. Numeric IPv4 in ANY
//! base (decimal `2130706433`, hex `0x7f000001`, octal `017700000001`,
//! short `127.1`) collapses to a u32 before classification, and IPv4-mapped
//! IPv6 (`::ffff:127.0.0.1`) unwraps to its v4 tail, so no encoding of a
//! non-public address slips past the dotted-quad check.

/// Fail-closed classification of an outbound URL host.
#[derive(Debug, PartialEq, Eq)]
pub enum GateError {
    /// Not absolute http/https (data:, file:, ftp:, relative, garbage).
    UnsupportedScheme,
    /// RFC-1918 / carrier-grade NAT range.
    PrivateAddress,
    /// Loopback (127/8 incl. numeric forms, ::1 incl. expanded, localhost,
    /// 0.0.0.0/8 which routes to localhost on most stacks).
    Loopback,
    /// Link-local (169.254/16, fe80::/10).
    LinkLocal,
    /// username[:password]@ before the host (credential leak shape).
    UserInfo,
    /// Longer than MAX_URL_LEN.
    TooLong,
}

/// Absolute cap on gated URL length.
pub const MAX_URL_LEN: usize = 2048;

/// Split authority from path/query/fragment. Scheme matched
/// case-insensitively (caller may hand `HTTPS://...`); the returned slices
/// borrow the ORIGINAL string so host case is preserved for diagnostics.
fn split_host(url: &str) -> Option<(&str, &str)> {
    let lower = url.to_ascii_lowercase();
    let rest = if lower.starts_with("http://") {
        &url["http://".len()..]
    } else if lower.starts_with("https://") {
        &url["https://".len()..]
    } else {
        return None;
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    Some((&rest[..end], &rest[end..]))
}

/// Strict dotted quad: exactly 4 decimal 8-bit parts.
fn is_ipv4(s: &str) -> bool {
    let p: Vec<&str> = s.split('.').collect();
    p.len() == 4
        && p.iter().all(|o| {
            !o.is_empty() && o.len() <= 3 && o.bytes().all(|b| b.is_ascii_digit()) && o.parse::<u8>().is_ok()
        })
}

/// Any host that LOOKS numeric but is not a strict dotted quad fails closed:
/// inet_pton-style stacks accept `0x7f.1`, `0177.0.0.1`, `127.1` etc, so a
/// host with digits/dots/hex-prefix the strict parser rejects is treated as
/// an evasion attempt, not a hostname. (parse_numeric_ipv4 below still
/// classifies the KNOWN numeric forms so the error KIND stays precise.)
fn looks_numeric(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h.is_empty() || h.contains(':') || h.contains('/') || h.contains('%') {
        return false;
    }
    // Character gate FIRST (hex digits + dots + at most one 0x marker):
    // anything else (letters like 'example') is a hostname, not a number.
    if !h.bytes().all(|b| b.is_ascii_hexdigit() || b == b'.' || b == b'x') {
        return false;
    }
    if h.matches("0x").count() > 1 || h.contains("xx") {
        return true; // garbage hex-ish: numeric-shaped, unparseable -> closed
    }
    let body = h.strip_prefix("0x").unwrap_or(&h).replace('.', "");
    if body.is_empty() {
        return true; // "0x", "0.0.0."-degenerate: numeric-shaped
    }
    // Pure digit string (any base's digits) or hex-digit string with 0x
    // marker or over-long digit runs no hostname label allows.
    body.bytes().all(|b| b.is_ascii_digit())
        || (h.starts_with("0x") && body.bytes().all(|b| b.is_ascii_hexdigit()))
        || body.len() > 15 && body.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Decimal/hex/octal IPv4 forms collapsing to one u32 (inet_aton rules):
/// `2130706433`, `0x7f000001`, `017700000001`, `127.1`, `10.0.1`.
/// Per-part exotic bases (`0x7f.1`) are NOT parsed here -- looks_numeric
/// already failed them closed; this only classifies clean forms.
fn parse_numeric_ipv4(host: &str) -> Option<u32> {
    let h = host.trim_end_matches('.');
    if h.is_empty() {
        return None;
    }
    if !h.contains('.') {
        // Single number: decimal, 0x-hex, or 0-octal.
        let (digits, radix) = if let Some(r) = h.strip_prefix("0x").or_else(|| h.strip_prefix("0X")) {
            (r, 16)
        } else if h.len() > 1 && h.starts_with('0') {
            (h, 8)
        } else {
            (h, 10)
        };
        if digits.is_empty() {
            return None;
        }
        let ok = match radix {
            16 => digits.bytes().all(|b| b.is_ascii_hexdigit()),
            8 => digits.bytes().all(|b| matches!(b, b'0'..=b'7')),
            _ => digits.bytes().all(|b| b.is_ascii_digit()),
        };
        if !ok {
            return None;
        }
        return u32::from_str_radix(digits, radix).ok();
    }
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() < 2 || parts.len() > 4 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    // Exotic per-part bases fail closed (handled by looks_numeric upstream).
    for p in &parts {
        if p.len() > 1 && (p.starts_with("0x") || p.starts_with("0X") || p.starts_with('0')) {
            return None;
        }
        if !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let n = parts.len();
    let vals: Vec<u32> = parts.iter().map(|p| p.parse().unwrap_or(u32::MAX)).collect();
    if vals.iter().take(n - 1).any(|v| *v > 255) {
        return None;
    }
    // inet_aton: first n-1 parts are top octets, last part fills the rest.
    let max_last: u32 = match n {
        2 => 1 << 24,
        3 => 1 << 16,
        _ => 256,
    };
    if vals[n - 1] >= max_last {
        return None;
    }
    let mut ip: u32 = 0;
    for (i, v) in vals.iter().enumerate() {
        let shift = if i == n - 1 { 0 } else { 8 * (3 - i) };
        ip |= v << shift;
    }
    Some(ip)
}

/// Classify 4 octets; None = public.
fn classify_v4(o: [u8; 4]) -> Option<GateError> {
    if o[0] == 127 || o[0] == 0 {
        // 0.0.0.0/8 routes to localhost on most stacks: loopback, not private.
        return Some(GateError::Loopback);
    }
    if o[0] == 169 && o[1] == 254 {
        return Some(GateError::LinkLocal);
    }
    if o[0] == 10
        || (o[0] == 172 && (16..=31).contains(&o[1]))
        || (o[0] == 192 && o[1] == 168)
        || (o[0] == 100 && (64..=127).contains(&o[1]))
    {
        return Some(GateError::PrivateAddress);
    }
    None
}

/// Normalize an IPv6 literal: lowercase, drop `%scope`, unwrap mapped
/// `::ffff:a.b.c.d` to `v4:<tail>`.
fn normalize_v6(host: &str) -> String {
    let h = host.split('%').next().unwrap_or(host).to_ascii_lowercase();
    for prefix in ["0:0:0:0:0:ffff:", "::ffff:"] {
        if let Some(tail) = h.strip_prefix(prefix) {
            if tail.contains('.') {
                return format!("v4:{tail}");
            }
        }
    }
    h
}

/// True for ::1 in ANY textual form: compressed, expanded
/// (0:0:0:0:0:0:0:1), mixed case. Parses hextets instead of char-matching.
fn is_loopback_v6(h: &str) -> bool {
    // Expand :: shorthand, then expect exactly 8 zero hextets + final 1.
    let expanded = if let Some((l, r)) = h.split_once("::") {
        if l.contains("::") || r.contains("::") {
            return false;
        }
        let ln = if l.is_empty() { 0 } else { l.split(':').count() };
        let rn = if r.is_empty() { 0 } else { r.split(':').count() };
        if ln + rn > 7 {
            return false;
        }
        let mut out = String::new();
        if !l.is_empty() {
            out.push_str(l);
            out.push(':');
        }
        for _ in 0..(8 - ln - rn) {
            out.push_str("0:");
        }
        if !r.is_empty() {
            out.push_str(r);
        } else {
            out.pop();
        }
        out
    } else {
        h.to_string()
    };
    let parts: Vec<&str> = expanded.split(':').collect();
    parts.len() == 8
        && parts[..7].iter().all(|p| p.trim_start_matches('0').is_empty())
        && parts[7].trim_start_matches('0') == "1"
}

/// fe80::/10: first hextet fe80..febf.
fn is_linklocal_v6(h: &str) -> bool {
    let first = h.split([':', '%']).next().unwrap_or("");
    first.len() == 4
        && first.bytes().all(|b| b.is_ascii_hexdigit())
        && {
            let lo = first.to_ascii_lowercase();
            lo.starts_with("fe")
                && matches!(
                    lo.as_str(),
                    "fe80" | "fe81" | "fe82" | "fe83" | "fe84" | "fe85" | "fe86" | "fe87"
                        | "fe88" | "fe89" | "fe8a" | "fe8b" | "fe8c" | "fe8d" | "fe8e"
                        | "fe8f" | "fea0" | "fea1" | "fea2" | "fea3" | "fea4" | "fea5"
                        | "fea6" | "fea7" | "fea8" | "fea9" | "feaa" | "feab" | "feac"
                        | "fead" | "feae" | "feaf" | "feb0" | "feb1" | "feb2" | "feb3"
                        | "feb4" | "feb5" | "feb6" | "feb7" | "feb8" | "feb9" | "feba"
                        | "febb" | "febc" | "febd" | "febe" | "febf"
                )
        }
}

/// Unique-local fc00::/7: first hextet fc.. or fd...
fn is_uniquelocal_v6(h: &str) -> bool {
    let first = h.split([':', '%']).next().unwrap_or("").to_ascii_lowercase();
    first.len() == 4
        && first.bytes().all(|b| b.is_ascii_hexdigit())
        && (first.starts_with("fc") || first.starts_with("fd"))
}

/// Fail-closed gate for one outbound URL. Ok(()) = public http/https host.
pub fn validate_public_url(url: &str) -> Result<(), GateError> {
    if url.len() > MAX_URL_LEN {
        return Err(GateError::TooLong);
    }
    // Any control byte or ASCII space: smuggling shape, not a URL.
    if url.bytes().any(|b| b <= 0x20 || b == 0x7f) {
        return Err(GateError::UnsupportedScheme);
    }
    // Percent-encoded host tricks (`%31%32%37...`): the decoded bytes never
    // reach our classifier, so any % in the authority fails closed.
    let (auth, _) = split_host(url).ok_or(GateError::UnsupportedScheme)?;
    if auth.is_empty() || auth.contains('%') {
        return Err(GateError::UnsupportedScheme);
    }
    if auth.contains('@') {
        return Err(GateError::UserInfo);
    }
    // Bracketed v6 (+ optional zone/port). Unclosed bracket, stray `]`, or
    // trailing garbage fails closed.
    let bare: &str = if let Some(rest) = auth.strip_prefix('[') {
        let (inside, after) = rest.split_once(']').ok_or(GateError::UnsupportedScheme)?;
        if inside.is_empty() {
            return Err(GateError::UnsupportedScheme);
        }
        if !after.is_empty() {
            let port = after.strip_prefix(':').ok_or(GateError::UnsupportedScheme)?;
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return Err(GateError::UnsupportedScheme);
            }
        }
        inside
    } else {
        if auth.contains(']') {
            return Err(GateError::UnsupportedScheme);
        }
        if auth.matches(':').count() == 1 {
            let (h, port) = auth.split_once(':').unwrap();
            if h.is_empty() || port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return Err(GateError::UnsupportedScheme);
            }
            h
        } else if auth.contains(':') {
            // Unbracketed colons: not valid host:port. Fail closed.
            return Err(GateError::UnsupportedScheme);
        } else {
            auth
        }
    };
    if bare.is_empty() {
        return Err(GateError::UnsupportedScheme);
    }
    let hl = bare.to_ascii_lowercase();
    if hl == "localhost" || hl.ends_with(".localhost") {
        return Err(GateError::Loopback);
    }
    // Trailing-dot FQDN normalizes first (127.0.0.1. is still loopback).
    let canon = hl.trim_end_matches('.');
    if canon.is_empty() {
        return Err(GateError::UnsupportedScheme);
    }
    // Numeric IPv4 in any base collapses to u32 first.
    if let Some(n) = parse_numeric_ipv4(canon) {
        if let Some(e) = classify_v4(n.to_be_bytes()) {
            return Err(e);
        }
        return Ok(());
    }
    // Numeric-LOOKING but unparseable (0x7f.1, 0177.0.0.1, 1.2.3.4.5):
    // a real resolver may still route these somewhere private. Fail closed.
    if looks_numeric(canon) {
        return Err(GateError::PrivateAddress);
    }
    // IPv6 literal (colon survived bracket strip).
    if bare.contains(':') {
        let norm = normalize_v6(bare);
        if let Some(v4tail) = norm.strip_prefix("v4:") {
            if let Some(n) = parse_numeric_ipv4(v4tail) {
                if let Some(e) = classify_v4(n.to_be_bytes()) {
                    return Err(e);
                }
                return Ok(());
            }
            return Err(GateError::UnsupportedScheme);
        }
        if is_loopback_v6(&norm) {
            return Err(GateError::Loopback);
        }
        if is_linklocal_v6(&norm) {
            return Err(GateError::LinkLocal);
        }
        if is_uniquelocal_v6(&norm) {
            return Err(GateError::PrivateAddress);
        }
        return Ok(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allows_public() {
        for u in [
            "https://example.com",
            "https://openrouter.ai/x",
            "http://192.0.2.1",
            "HTTPS://EXAMPLE.COM/x",
            "https://example.com:8443/x",
            "http://[2001:db8::1]/",
        ] {
            assert!(validate_public_url(u).is_ok(), "should allow {u}");
        }
    }
    #[test]
    fn rejects_bad() {
        for u in [
            "http://127.0.0.1",
            "http://127.0.0.1.",
            "https://localhost",
            "https://LOCALHOST/x",
            "https://x.localhost/",
            "https://169.254.169.254/latest/meta-data/",
            "http://10.0.0.1",
            "http://100.64.0.1",
            "http://100.127.255.255",
            "http://172.16.5.4",
            "http://172.31.255.255",
            "http://192.168.1.1",
            "http://0.0.0.0/",
            "http://[::1]/",
            "http://[0:0:0:0:0:0:0:1]/",
            "http://[0:0:0:0:0:0:0:01]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:10.0.0.1]/",
            "http://[::FFFF:192.168.0.1]/",
            "http://[fe80::1]/",
            "http://[FEBF:1234::1]/",
            "http://[fc00::1]/",
            "http://[fd12:3456::1]/",
            "http://2130706433/",
            "http://0x7f000001/",
            "http://0X7F000001/",
            "http://0xA000001/",
            "http://017700000001/",
            "http://3232235777/",
            "http://127.1/",
            "http://10.0.1/",
            "http://0x7f.1/",
            "http://0177.0.0.1/",
            "http://1.2.3.4.5/",
            "http://%31%32%37.0.0.1/",
            "data:text/plain,http:",
            "file:///etc/passwd",
            "ftp://example.com/x",
            "https://user:pass@example.com",
            "http://example.com:abc/",
            "http://a:b:c/",
            "http://[::1",
            "http://]/",
            "http://",
            "https://exa mple.com/",
            "https://example.com/\nHeader: x",
        ] {
            assert!(validate_public_url(u).is_err(), "should reject {u}");
        }
        let long = "https://example.com/".to_string() + &"a".repeat(2048);
        assert_eq!(validate_public_url(&long), Err(GateError::TooLong));
    }
    #[test]
    fn error_kinds() {
        assert_eq!(validate_public_url("http://127.0.0.1"), Err(GateError::Loopback));
        assert_eq!(validate_public_url("http://2130706433"), Err(GateError::Loopback));
        assert_eq!(validate_public_url("http://0x7f000001"), Err(GateError::Loopback));
        assert_eq!(validate_public_url("http://017700000001"), Err(GateError::Loopback));
        assert_eq!(validate_public_url("http://127.1"), Err(GateError::Loopback));
        assert_eq!(validate_public_url("http://0.0.0.0"), Err(GateError::Loopback));
        assert_eq!(validate_public_url("http://0x7f.1"), Err(GateError::PrivateAddress));
        assert_eq!(validate_public_url("http://10.1.2.3"), Err(GateError::PrivateAddress));
        assert_eq!(validate_public_url("http://0xA000001"), Err(GateError::PrivateAddress));
        assert_eq!(validate_public_url("http://100.64.0.1"), Err(GateError::PrivateAddress));
        assert_eq!(validate_public_url("http://[::ffff:10.0.0.1]/"), Err(GateError::PrivateAddress));
        assert_eq!(validate_public_url("http://[fc00::1]/"), Err(GateError::PrivateAddress));
        assert_eq!(validate_public_url("https://169.254.1.1"), Err(GateError::LinkLocal));
        assert_eq!(validate_public_url("http://[fe80::1]/"), Err(GateError::LinkLocal));
        assert_eq!(validate_public_url("http://[::1]/"), Err(GateError::Loopback));
        assert_eq!(validate_public_url("http://[0:0:0:0:0:0:0:1]/"), Err(GateError::Loopback));
        assert_eq!(validate_public_url("data:x"), Err(GateError::UnsupportedScheme));
        assert_eq!(validate_public_url("https://u@example.com"), Err(GateError::UserInfo));
    }
}
