//! Ports the small helpers at the top of avatar_export.cpp (category names, slot labels, name sanitizing) plus C parsing.

use crate::manifest::category;

const CATEGORY_NAMES: [(u32, &str); 14] = [
    (category::HEAD, "Head"),
    (category::BODY, "Body"),
    (category::HAIR, "Hair"),
    (category::TOP, "Top"),
    (category::BOTTOM, "Bottom"),
    (category::SHOES, "Shoes"),
    (category::HAT, "Hat"),
    (category::GLOVES, "Gloves"),
    (category::GLASSES, "Glasses"),
    (category::WRISTWEAR, "Wristwear"),
    (category::EARRINGS, "Earrings"),
    (category::RING, "Ring"),
    (category::PROP, "Prop"),
    (category::ANIMATION, "Animation"),
];

/// CategoryNames: `Top|Bottom|...`, or `(none)`.
pub fn category_names(categories: u32) -> String {
    let names: Vec<&str> = CATEGORY_NAMES
        .iter()
        .filter(|(bit, _)| categories & bit != 0)
        .map(|(_, name)| *name)
        .collect();
    if names.is_empty() {
        "(none)".to_owned()
    } else {
        names.join("|")
    }
}

/// SlotLabel: short slot name used for material and mesh names.
pub fn slot_label(categories: u32) -> String {
    let garments = category::TOP | category::BOTTOM | category::SHOES | category::GLOVES;
    let garment_bits = (0..13)
        .filter(|b| categories & (1u32 << b) != 0 && (1u32 << b) & garments != 0)
        .count();
    if categories & category::BODY != 0 {
        return "Body".into();
    }
    if categories & category::HEAD != 0 {
        return "Head".into();
    }
    if garment_bits >= 3 {
        return "Costume".into();
    }
    const ORDER: [u32; 11] = [
        category::TOP,
        category::BOTTOM,
        category::SHOES,
        category::HAIR,
        category::HAT,
        category::GLOVES,
        category::GLASSES,
        category::WRISTWEAR,
        category::EARRINGS,
        category::RING,
        category::PROP,
    ];
    for bit in ORDER {
        if categories & bit != 0 {
            if let Some((_, name)) = CATEGORY_NAMES.iter().find(|(b, _)| *b == bit) {
                return (*name).into();
            }
        }
    }
    "Item".into()
}

/// SanitizeName: ASCII alphanumerics, `_` and `-` kept, spaces become `_`, the rest dropped.
pub fn sanitize_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.bytes() {
        if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
            out.push(c as char);
        } else if c == b' ' {
            out.push('_');
        }
    }
    if out.is_empty() {
        out = "item".into();
    }
    out
}

/// ASCII-only lower-casing, as `std::tolower` in the C locale.
pub fn lower(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn skip_c_space(s: &str) -> &str {
    s.trim_start_matches([' ', '\t', '\n', '\r', '\x0b', '\x0c'])
}

/// C `atoi`: optional sign and leading digits, 0 when there are none; saturates instead of overflowing.
pub fn atoi(s: &str) -> i32 {
    let s = skip_c_space(s);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut v: i64 = 0;
    for c in digits.bytes() {
        if !c.is_ascii_digit() {
            break;
        }
        v = (v * 10 + i64::from(c - b'0')).min(i64::from(i32::MAX) + 1);
    }
    let v = if neg { -v } else { v };
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

/// Longest prefix of `s` that parses as a decimal float, with its value (C `strtod` without hex forms).
fn strtod_prefix(s: &str) -> Option<(f64, usize)> {
    let b = s.as_bytes();
    let mut i = 0;
    if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
        i += 1;
    }
    let mantissa_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    let mantissa = &s[mantissa_start..i];
    if mantissa.is_empty() || mantissa == "." {
        let rest = &s[mantissa_start..].to_ascii_lowercase();
        for word in ["infinity", "inf", "nan"] {
            if rest.starts_with(word) {
                let end = mantissa_start + word.len();
                return s[..end].parse::<f64>().ok().map(|v| (v, end));
            }
        }
        return None;
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        let mut j = i + 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        let exp_digits = j;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > exp_digits {
            i = j;
        }
    }
    s[..i].parse::<f64>().ok().map(|v| (v, i))
}

/// C `strtod` returning the value and whether the whole string was consumed.
pub fn strtod_full(s: &str) -> Option<f64> {
    let t = skip_c_space(s);
    match strtod_prefix(t) {
        Some((v, used)) if used == t.len() => Some(v),
        _ => None,
    }
}

/// C `atof`: the parsed prefix, 0 when there is none.
pub fn atof(s: &str) -> f64 {
    strtod_prefix(skip_c_space(s)).map(|(v, _)| v).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_parsing() {
        assert_eq!(atoi("512"), 512);
        assert_eq!(atoi("  -7x"), -7);
        assert_eq!(atoi("x"), 0);
        assert_eq!(strtod_full("0.35"), Some(0.35));
        assert_eq!(strtod_full("12"), Some(12.0));
        assert_eq!(strtod_full("mid"), None);
        assert_eq!(strtod_full("1e"), None);
        assert_eq!(atof("25deg"), 25.0);
    }

    #[test]
    fn names() {
        assert_eq!(
            sanitize_name("L.A. Noire 1940's Detective Suit"),
            "LA_Noire_1940s_Detective_Suit"
        );
        assert_eq!(sanitize_name("!!"), "item");
        assert_eq!(category_names(0x278), "Top|Bottom|Shoes|Hat|Wristwear");
        assert_eq!(category_names(0), "(none)");
        assert_eq!(slot_label(0x278), "Costume");
        assert_eq!(slot_label(category::HAIR), "Hair");
    }
}
