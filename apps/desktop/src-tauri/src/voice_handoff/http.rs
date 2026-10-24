//! Borrowed HTTP intake for the single-flight encrypted application handoff. This parser
//! never lowercases/copies attacker-controlled names or constructs a header map. Its caller
//! must own a paid header worker BEFORE allocating/filling the fixed scratch buffer.
//! Parsing is not authentication: verify the admission MAC before allocating/decrypting body.

pub(super) const HEADER_BYTES: usize = 4096;
pub(super) const BODY_BYTES: usize = 2048 + 16;
const HEADER_FIELDS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Method {
    Post,
    Preflight,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Refused;
/// All slices borrow the caller's charged scratch. A coalesced body prefix remains charged
/// with that same worker until copied into separately prepaid authenticated body custody.
pub(super) struct Head<'a> {
    pub method: Method,
    pub admission: Option<&'a str>,
    pub length: usize,
    pub body_prefix: &'a [u8],
}

fn once<'a>(slot: &mut Option<&'a str>, value: &'a str) -> Result<(), Refused> {
    if slot.replace(value).is_some() {
        Err(Refused)
    } else {
        Ok(())
    }
}
fn token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}
fn host(value: &str, port: u16) -> bool {
    let Some(number) = value.strip_prefix("127.0.0.1:") else {
        return false;
    };
    !number.is_empty()
        && !number.starts_with('0')
        && number.len() <= 5
        && number.bytes().all(|b| b.is_ascii_digit())
        && number.parse::<u16>() == Ok(port)
        && port != 0
}
fn requested_headers(value: &str) -> bool {
    let mut content = false;
    let mut admission = false;
    for field in value.split(',') {
        let field = field.trim_matches([' ', '\t']);
        if field.eq_ignore_ascii_case("content-type") && !content {
            content = true;
        } else if field.eq_ignore_ascii_case("x-voice-admission") && !admission {
            admission = true;
        } else {
            return false;
        }
    }
    content && admission
}

/// None means a bounded partial header, never permission to grow scratch. Unknown ordinary
/// headers are ignored without ownership/copy; all framing/CORS/admission fields are unique.
/// Reject ambiguous transfer encodings, folded fields, noncanonical lengths/Host and URLs.
pub(super) fn parse<'a>(
    scratch: &'a [u8],
    port: u16,
    expected_origin: &str,
) -> Result<Option<Head<'a>>, Refused> {
    if scratch.len() > HEADER_BYTES {
        return Err(Refused);
    }
    let Some(end) = scratch.windows(4).position(|part| part == b"\r\n\r\n") else {
        return if scratch.len() == HEADER_BYTES {
            Err(Refused)
        } else {
            Ok(None)
        };
    };
    let bytes = &scratch[..end];
    if !bytes.is_ascii() {
        return Err(Refused);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Refused)?;
    let mut lines = text.split("\r\n");
    let method = match lines.next() {
        Some("POST /voice/v2 HTTP/1.1") => Method::Post,
        Some("OPTIONS /voice/v2 HTTP/1.1") => Method::Preflight,
        _ => return Err(Refused),
    };
    let (
        mut authority,
        mut origin,
        mut length,
        mut admission,
        mut content_type,
        mut requested_method,
        mut allowed_headers,
    ) = (None, None, None, None, None, None, None);
    for (index, line) in lines.enumerate() {
        if index >= HEADER_FIELDS {
            return Err(Refused);
        }
        let (name, value) = line.split_once(':').ok_or(Refused)?;
        if name.is_empty()
            || !name.bytes().all(token)
            || !value
                .bytes()
                .all(|byte| byte == b'\t' || (32..=126).contains(&byte))
        {
            return Err(Refused);
        }
        let value = value.trim_matches([' ', '\t']);
        if name.eq_ignore_ascii_case("host") {
            once(&mut authority, value)?;
        } else if name.eq_ignore_ascii_case("origin") {
            once(&mut origin, value)?;
        } else if name.eq_ignore_ascii_case("content-length") {
            once(&mut length, value)?;
        } else if name.eq_ignore_ascii_case("content-type") {
            once(&mut content_type, value)?;
        } else if name.eq_ignore_ascii_case("x-voice-admission") {
            once(&mut admission, value)?;
        } else if name.eq_ignore_ascii_case("access-control-request-method") {
            once(&mut requested_method, value)?;
        } else if name.eq_ignore_ascii_case("access-control-request-headers") {
            once(&mut allowed_headers, value)?;
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            || name.eq_ignore_ascii_case("content-encoding")
        {
            return Err(Refused);
        }
    }
    if !authority.is_some_and(|value| host(value, port)) || origin != Some(expected_origin) {
        return Err(Refused);
    }
    let body_prefix = &scratch[end + 4..];
    let declared = match length {
        Some(value)
            if !value.is_empty()
                && value.len() <= 4
                && (value == "0" || !value.starts_with('0'))
                && value.bytes().all(|b| b.is_ascii_digit()) =>
        {
            value.parse::<usize>().map_err(|_| Refused)?
        }
        None if method == Method::Preflight => 0,
        _ => return Err(Refused),
    };
    match method {
        Method::Post => {
            if !(16..=BODY_BYTES).contains(&declared)
                || body_prefix.len() > declared
                || content_type
                    .is_none_or(|value| !value.eq_ignore_ascii_case("application/octet-stream"))
                || requested_method.is_some()
                || allowed_headers.is_some()
                || admission.is_none_or(|value| {
                    value.len() != 144
                        || !value
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
            {
                return Err(Refused);
            }
        }
        Method::Preflight => {
            if declared != 0
                || !body_prefix.is_empty()
                || admission.is_some()
                || content_type.is_some()
                || requested_method != Some("POST")
                || !allowed_headers.is_some_and(requested_headers)
            {
                return Err(Refused);
            }
        }
    }
    Ok(Some(Head {
        method,
        admission,
        length: declared,
        body_prefix,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    const PORT: u16 = 41234;
    const ORIGIN: &str = "http://tauri.localhost";
    fn post(extra: &str) -> String {
        format!("POST /voice/v2 HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nOrigin: {ORIGIN}\r\nContent-Length: 16\r\nContent-Type: application/octet-stream\r\nx-voice-admission: {}\r\n{extra}\r\n", "1".repeat(144))
    }
    #[test]
    fn fixed_fields_and_coalesced_body_borrow_original_scratch() {
        let mut request = post("User-Agent: fixed\r\n").into_bytes();
        request.extend_from_slice(&[255; 16]);
        let parsed = parse(&request, PORT, ORIGIN).unwrap().unwrap();
        assert_eq!(parsed.method, Method::Post);
        assert_eq!(parsed.length, 16);
        assert_eq!(parsed.body_prefix, &[255; 16]);
        let admission = parsed.admission.unwrap();
        assert_eq!(admission.len(), 144);
        assert!(admission.as_ptr() >= request.as_ptr());
        assert!(admission.as_ptr() < end_ptr(&request));
    }
    fn end_ptr(bytes: &[u8]) -> *const u8 {
        // No pointer arithmetic/unsafe: an in-bounds empty tail has the one-past-end address.
        bytes[bytes.len()..].as_ptr()
    }
    #[test]
    fn partial_header_never_requests_growing_storage() {
        let request = post("");
        for count in 0..request.len() {
            assert!(parse(&request.as_bytes()[..count], PORT, ORIGIN)
                .unwrap()
                .is_none());
        }
        assert!(parse(&vec![b'x'; HEADER_BYTES], PORT, ORIGIN).is_err());
        assert!(parse(&vec![b'x'; HEADER_BYTES + 1], PORT, ORIGIN).is_err());
    }
    #[test]
    fn duplicated_or_ambiguous_framing_and_cors_fields_refuse() {
        for extra in [
            "Host: 127.0.0.1:41234",
            "oRiGiN: http://tauri.localhost",
            "content-length: 16",
            "Content-Type: application/octet-stream",
            "x-voice-admission: 1",
            "Transfer-Encoding: chunked",
            "Content-Encoding: gzip",
            "Access-Control-Request-Method: POST",
            " folded: value",
            "bad name: x",
            "X-Null: \0",
        ] {
            assert!(
                parse(post(&format!("{extra}\r\n")).as_bytes(), PORT, ORIGIN).is_err(),
                "{extra:?}"
            );
        }
    }
    #[test]
    fn wrong_origin_host_method_path_and_length_refuse() {
        let original = post("");
        for (old, new) in [
            ("http://tauri.localhost", "http://localhost"),
            ("127.0.0.1:41234", "localhost:41234"),
            ("127.0.0.1:41234", "127.0.0.1:041234"),
            ("Content-Length: 16", "Content-Length: 016"),
            ("Content-Length: 16", "Content-Length: +16"),
            ("Content-Length: 16", "Content-Length: 2065"),
            ("Content-Length: 16", "Content-Length: 15"),
            ("POST /voice/v2", "GET /voice/v2"),
            ("POST /voice/v2", "POST /voice/v2?x=1"),
            ("POST /voice/v2", "POST http://127.0.0.1:41234/voice/v2"),
        ] {
            assert!(
                parse(original.replace(old, new).as_bytes(), PORT, ORIGIN).is_err(),
                "{old} -> {new}"
            );
        }
        assert!(parse(original.as_bytes(), PORT + 1, ORIGIN).is_err());
        assert!(parse(
            original
                .replace(&"1".repeat(144), &"A".repeat(144))
                .as_bytes(),
            PORT,
            ORIGIN
        )
        .is_err());
    }
    #[test]
    fn preflight_is_exact_and_has_no_body_or_admission() {
        let preflight = format!("OPTIONS /voice/v2 HTTP/1.1\r\nHost: 127.0.0.1:{PORT}\r\nOrigin: {ORIGIN}\r\nAccess-Control-Request-Method: POST\r\nAccess-Control-Request-Headers: X-Voice-Admission, Content-Type\r\n\r\n");
        let parsed = parse(preflight.as_bytes(), PORT, ORIGIN).unwrap().unwrap();
        assert_eq!(parsed.method, Method::Preflight);
        assert_eq!(parsed.length, 0);
        for invalid in [
            preflight.replace("X-Voice-Admission, Content-Type", "content-type"),
            preflight.replace(
                "X-Voice-Admission, Content-Type",
                "x-voice-admission,content-type,x-other",
            ),
            preflight.replace(
                "X-Voice-Admission, Content-Type",
                "x-voice-admission,content-type,content-type",
            ),
            preflight.replace("Method: POST", "Method: GET"),
            preflight.clone() + "x",
        ] {
            assert!(parse(invalid.as_bytes(), PORT, ORIGIN).is_err());
        }
    }
    #[test]
    fn many_or_long_unknown_names_are_borrowed_and_work_is_capped() {
        assert!(parse(
            post(&format!("{}: x\r\n", "a".repeat(3000))).as_bytes(),
            PORT,
            ORIGIN
        )
        .is_ok());
        let too_many = (0..HEADER_FIELDS)
            .map(|index| format!("X-{index}: x\r\n"))
            .collect::<String>();
        assert!(parse(post(&too_many).as_bytes(), PORT, ORIGIN).is_err());
    }
    #[test]
    fn excess_coalesced_body_and_missing_critical_fields_refuse() {
        let request = post("");
        assert!(parse((request.clone() + &"x".repeat(17)).as_bytes(), PORT, ORIGIN).is_err());
        for line in [
            "Host: 127.0.0.1:41234\r\n",
            "Origin: http://tauri.localhost\r\n",
            "Content-Length: 16\r\n",
            "Content-Type: application/octet-stream\r\n",
        ] {
            assert!(parse(request.replace(line, "").as_bytes(), PORT, ORIGIN).is_err());
        }
    }
}
