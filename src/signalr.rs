pub const SIGNALR_SEP: char = '\x1e';

pub fn encode_message(msg: &str) -> String {
    format!("{}{}", msg, SIGNALR_SEP)
}

pub fn decode_messages(raw: &str) -> Vec<String> {
    raw.split(SIGNALR_SEP)
        .map(|s| s.trim().to_owned())
         .filter(|s| !s.is_empty())
         .collect()
}

/// Escape non-ASCII characters as \uXXXX to match Python's json.dumps(ensure_ascii=True).
/// Only needed for BMP characters (U+0000..U+FFFF); surrogates handled for completeness.
pub fn ensure_ascii_json(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let code = c as u32;
            if code < 0x10000 {
                out.push_str(&format!("\\u{:04x}", code));
            } else {
                let c = code - 0x10000;
                let hi = 0xD800 | (c >> 10);
                let lo = 0xDC00 | (c & 0x3FF);
                out.push_str(&format!("\\u{:04x}\\u{:04x}", hi, lo));
            }
        }
    }
    out
}
