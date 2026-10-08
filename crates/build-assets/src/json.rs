// Python `json.dumps(value, indent=2)` output with CRLF line ends, the format the reference assets were written in.

pub enum Json {
    Str(String),
    Int(i64),
    Float(f64),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl From<&str> for Json {
    fn from(s: &str) -> Self {
        Json::Str(s.into())
    }
}

impl From<String> for Json {
    fn from(s: String) -> Self {
        Json::Str(s)
    }
}

pub fn obj<const N: usize>(fields: [(&str, Json); N]) -> Json {
    Json::Obj(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// ensure_ascii escaping: non-ASCII as `\uXXXX`, astral characters as surrogate pairs.
fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='\u{7f}' => out.push(c),
            _ => {
                let mut units = [0u16; 2];
                for u in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{u:04x}"));
                }
            }
        }
    }
    out.push('"');
}

/// Python float repr; Rust's `{:?}` agrees for the finite, mid-range values written here.
fn float(v: f64) -> String {
    format!("{v:?}")
}

fn write(out: &mut String, v: &Json, depth: usize) {
    let pad = |out: &mut String, d: usize| {
        out.push('\n');
        out.push_str(&"  ".repeat(d));
    };
    match v {
        Json::Str(s) => string(out, s),
        Json::Int(i) => out.push_str(&i.to_string()),
        Json::Float(f) => out.push_str(&float(*f)),
        Json::Arr(items) if items.is_empty() => out.push_str("[]"),
        Json::Obj(items) if items.is_empty() => out.push_str("{}"),
        Json::Arr(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                pad(out, depth + 1);
                write(out, item, depth + 1);
            }
            pad(out, depth);
            out.push(']');
        }
        Json::Obj(items) => {
            out.push('{');
            for (i, (k, item)) in items.iter().enumerate() {
                if i != 0 {
                    out.push(',');
                }
                pad(out, depth + 1);
                string(out, k);
                out.push_str(": ");
                write(out, item, depth + 1);
            }
            pad(out, depth);
            out.push('}');
        }
    }
}

/// The dump with CRLF line ends, plus a final CRLF when `trailing_newline` is set.
pub fn dump(v: &Json, trailing_newline: bool) -> String {
    let mut out = String::new();
    write(&mut out, v, 0);
    if trailing_newline {
        out.push('\n');
    }
    out.replace('\n', "\r\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_python_layout() {
        let v = obj([
            ("a", Json::Arr(vec![Json::Int(1), Json::Float(20.0)])),
            ("b", "x \u{b7} y".into()),
            ("c", Json::Arr(Vec::new())),
        ]);
        assert_eq!(
            dump(&v, false),
            "{\r\n  \"a\": [\r\n    1,\r\n    20.0\r\n  ],\r\n  \"b\": \"x \\u00b7 y\",\r\n  \"c\": []\r\n}"
        );
    }
}
