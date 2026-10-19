//! Byte-level MIME helpers for signed and encrypted mail, where every byte counts: a
//! signature covers the signed part exactly as it was sent. Shared with the OpenPGP crate.

/// The header block (with its final CRLF) and the body of a MIME entity in CRLF form.
pub fn split_entity(raw: &[u8]) -> (&[u8], &[u8]) {
    if raw.starts_with(b"\r\n") {
        return (&[], &raw[2..]);
    }
    match find(raw, b"\r\n\r\n") {
        Some(i) => (&raw[..i + 2], &raw[i + 4..]),
        None => (raw, &[]),
    }
}

/// The unfolded value of the first header field called `name` (lower case).
pub fn header(head: &[u8], name: &[u8]) -> Option<String> {
    fields(head).into_iter().find_map(|f| {
        let colon = f.iter().position(|b| *b == b':')?;
        if !f[..colon].trim_ascii().eq_ignore_ascii_case(name) {
            return None;
        }
        let value = String::from_utf8_lossy(&f[colon + 1..]).replace("\r\n", "");
        Some(value.trim().to_string())
    })
}

/// A parameter of a header value such as `boundary="abc"`.
pub fn parameter(value: &str, name: &str) -> Option<String> {
    for part in value.split(';').skip(1) {
        let (key, v) = part.split_once('=')?;
        if key.trim().eq_ignore_ascii_case(name) {
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v);
            return (!v.is_empty() && v.len() <= 200).then(|| v.to_string());
        }
    }
    None
}

/// The header fields of `raw` that stay on the outside of a signed or encrypted message (all
/// but the content headers and MIME-Version), and the MIME entity inside it: the content
/// headers and the body, in CRLF form, ending with a line break.
pub fn split_message(raw: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let raw = crlf(raw);
    let (head, body) = match find(&raw, b"\r\n\r\n") {
        Some(i) => (&raw[..i + 2], &raw[i + 4..]),
        None => (raw.as_slice(), &b""[..]),
    };
    let (mut outer, mut inner) = (Vec::new(), Vec::new());
    for field in fields(head) {
        let name = field.split(|b| *b == b':').next().unwrap_or_default().to_ascii_lowercase();
        if name.starts_with(b"content-") {
            inner.extend_from_slice(field);
        } else if name != b"mime-version" {
            outer.extend_from_slice(field);
        }
    }
    if inner.is_empty() {
        inner.extend_from_slice(b"Content-Type: text/plain; charset=us-ascii\r\n");
    }
    inner.extend_from_slice(b"\r\n");
    inner.extend_from_slice(body);
    if !inner.ends_with(b"\r\n") {
        inner.extend_from_slice(b"\r\n");
    }
    (outer, inner)
}

/// Every line ending as CRLF, the canonical form a signature covers.
pub fn crlf(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len() + raw.len() / 40);
    let mut prev = 0u8;
    for &b in raw {
        if b == b'\n' && prev != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        prev = b;
    }
    out
}

pub fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// The header fields of `head`, each with its folded continuation lines and final CRLF.
pub fn fields(head: &[u8]) -> Vec<&[u8]> {
    let mut out: Vec<&[u8]> = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < head.len() {
        if head[i] == b'\n' {
            let next = head.get(i + 1).copied();
            if !matches!(next, Some(b' ' | b'\t')) {
                out.push(&head[start..=i]);
                start = i + 1;
            }
        }
        i += 1;
    }
    if start < head.len() {
        out.push(&head[start..]);
    }
    out
}

/// The first two parts of the multipart `body` with `boundary`: the first part's entity
/// byte for byte (without the line break before the next delimiter, which belongs to it), and
/// the second part's entity.
pub fn two_parts<'a>(body: &'a [u8], boundary: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let mut delimiter = b"\r\n--".to_vec();
    delimiter.extend_from_slice(boundary);
    // The first delimiter may open the body, without a line break before it.
    let first = if body.starts_with(&delimiter[2..]) { 0 } else { find(body, &delimiter)? + 2 };
    let after = first + delimiter.len() - 2;
    let start = after + find(&body[after..], b"\r\n")? + 2;
    let end = start + find(&body[start..], &delimiter)?;
    let next = end + delimiter.len();
    let second = next + find(&body[next..], b"\r\n")? + 2;
    let close = second + find(&body[second..], &delimiter)?;
    Some((&body[start..end], &body[second..close]))
}
