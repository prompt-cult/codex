/// A hand-rolled, forward-only JSON reader. Not a general-purpose JSON
/// library: it supports exactly the shapes `threadbox.ir.v1` uses —
/// objects, arrays, strings, and integers — and nothing else. See
/// `IR.md`: "the reader is tactical ... regenerated when the graph
/// changes", never a serialization crate reached for "to clean it up".
use crate::error::{fail_msg, ParseError};

/// One parsed JSON value. `Int` is the only number shape the envelope
/// ever carries (`i`, `width`, `bound`, `body`, `contextWindow`).
pub enum JsonValue {
    Str(String),
    Int(i64),
    Arr(Vec<JsonValue>),
    Obj(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            JsonValue::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Arr(items) => Some(items.as_slice()),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&[(String, JsonValue)]> {
        match self {
            JsonValue::Obj(fields) => Some(fields.as_slice()),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        self.as_obj()?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
}

/// Parse a complete JSON document. Trailing bytes after the top-level
/// value (other than whitespace) are an error.
pub fn parse(src: &str) -> Result<JsonValue, ParseError> {
    let bytes = src.as_bytes();
    let mut pos = 0usize;
    skip_ws(bytes, &mut pos);
    let value = parse_value(bytes, &mut pos)?;
    skip_ws(bytes, &mut pos);
    if pos != bytes.len() {
        return Err(fail_msg(format!(
            "trailing content at byte offset {pos}; expected end of input"
        )));
    }
    Ok(value)
}

fn skip_ws(src: &[u8], pos: &mut usize) {
    while *pos < src.len() && matches!(src[*pos], b' ' | b'\t' | b'\n' | b'\r') {
        *pos += 1;
    }
}

fn peek(src: &[u8], pos: usize) -> Result<u8, ParseError> {
    src.get(pos).copied().ok_or_else(|| {
        fail_msg(format!("unexpected end of input at byte offset {pos}; expected more JSON content"))
    })
}

fn expect(src: &[u8], pos: &mut usize, byte: u8) -> Result<(), ParseError> {
    let actual = peek(src, *pos)?;
    if actual != byte {
        return Err(fail_msg(format!(
            "byte at offset {pos} is '{}'; expected '{}'",
            actual as char, byte as char
        )));
    }
    *pos += 1;
    Ok(())
}

fn parse_value(src: &[u8], pos: &mut usize) -> Result<JsonValue, ParseError> {
    skip_ws(src, pos);
    match peek(src, *pos)? {
        b'"' => Ok(JsonValue::Str(parse_string(src, pos)?)),
        b'{' => parse_object(src, pos),
        b'[' => parse_array(src, pos),
        b'-' | b'0'..=b'9' => Ok(JsonValue::Int(parse_integer(src, pos)?)),
        other => Err(fail_msg(format!(
            "byte at offset {pos} is '{}'; expected a JSON string, object, array, or integer",
            other as char
        ))),
    }
}

fn parse_object(src: &[u8], pos: &mut usize) -> Result<JsonValue, ParseError> {
    expect(src, pos, b'{')?;
    let mut fields = Vec::new();
    skip_ws(src, pos);
    if peek(src, *pos)? == b'}' {
        *pos += 1;
        return Ok(JsonValue::Obj(fields));
    }
    loop {
        skip_ws(src, pos);
        let key = parse_string(src, pos)?;
        skip_ws(src, pos);
        expect(src, pos, b':')?;
        let value = parse_value(src, pos)?;
        fields.push((key, value));
        skip_ws(src, pos);
        match peek(src, *pos)? {
            b',' => {
                *pos += 1;
                continue;
            }
            b'}' => {
                *pos += 1;
                break;
            }
            other => {
                return Err(fail_msg(format!(
                    "byte at offset {pos} is '{}'; expected ',' or '}}' in object",
                    other as char
                )))
            }
        }
    }
    Ok(JsonValue::Obj(fields))
}

fn parse_array(src: &[u8], pos: &mut usize) -> Result<JsonValue, ParseError> {
    expect(src, pos, b'[')?;
    let mut items = Vec::new();
    skip_ws(src, pos);
    if peek(src, *pos)? == b']' {
        *pos += 1;
        return Ok(JsonValue::Arr(items));
    }
    loop {
        let value = parse_value(src, pos)?;
        items.push(value);
        skip_ws(src, pos);
        match peek(src, *pos)? {
            b',' => {
                *pos += 1;
                continue;
            }
            b']' => {
                *pos += 1;
                break;
            }
            other => {
                return Err(fail_msg(format!(
                    "byte at offset {pos} is '{}'; expected ',' or ']' in array",
                    other as char
                )))
            }
        }
    }
    Ok(JsonValue::Arr(items))
}

fn parse_string(src: &[u8], pos: &mut usize) -> Result<String, ParseError> {
    expect(src, pos, b'"')?;
    let mut out = String::new();
    loop {
        let c = peek(src, *pos)?;
        *pos += 1;
        match c {
            b'"' => break,
            b'\\' => {
                let esc = peek(src, *pos)?;
                *pos += 1;
                match esc {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if *pos + 4 > src.len() {
                            return Err(fail_msg(format!(
                                "unicode escape at offset {pos} is truncated; expected 4 hex digits"
                            )));
                        }
                        let hex = std::str::from_utf8(&src[*pos..*pos + 4]).map_err(|_| {
                            fail_msg(format!("unicode escape at offset {pos} is not valid UTF-8"))
                        })?;
                        let code = u32::from_str_radix(hex, 16).map_err(|_| {
                            fail_msg(format!("unicode escape \\u{hex} at offset {pos} is not valid hex"))
                        })?;
                        if let Some(ch) = char::from_u32(code) {
                            out.push(ch);
                        }
                        *pos += 4;
                    }
                    other => {
                        return Err(fail_msg(format!(
                            "escape sequence '\\{}' at offset {pos} is not recognized",
                            other as char
                        )))
                    }
                }
            }
            _ => {
                // Reconstitute the UTF-8 codepoint starting at c.
                let start = *pos - 1;
                let width = utf8_width(c);
                let end = start + width;
                if end > src.len() {
                    return Err(fail_msg(format!(
                        "string starting before offset {start} is truncated inside a UTF-8 sequence"
                    )));
                }
                let s = std::str::from_utf8(&src[start..end]).map_err(|_| {
                    fail_msg(format!("string at offset {start} contains invalid UTF-8"))
                })?;
                out.push_str(s);
                *pos = end;
            }
        }
    }
    Ok(out)
}

fn utf8_width(first_byte: u8) -> usize {
    if first_byte & 0x80 == 0 {
        1
    } else if first_byte & 0xE0 == 0xC0 {
        2
    } else if first_byte & 0xF0 == 0xE0 {
        3
    } else {
        4
    }
}

fn parse_integer(src: &[u8], pos: &mut usize) -> Result<i64, ParseError> {
    let start = *pos;
    if peek(src, *pos)? == b'-' {
        *pos += 1;
    }
    let digits_start = *pos;
    while *pos < src.len() && src[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == digits_start {
        return Err(fail_msg(format!(
            "number at offset {start} has no digits; expected an integer"
        )));
    }
    let text = std::str::from_utf8(&src[start..*pos]).unwrap();
    text.parse::<i64>().map_err(|_| {
        fail_msg(format!("number '{text}' at offset {start} is not a valid integer"))
    })
}
