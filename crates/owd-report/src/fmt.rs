//! How a report prints numbers and text. A report prints what the document holds, rounded for
//! reading; it never computes a result of its own.

use serde_json::Value;

/// `x` with `places` decimals and a thousands separator (`1,234.5`). `-0` prints as `0`.
pub fn num(x: f64, places: usize) -> String {
    let s = format!("{:.*}", places, x);
    let (sign, s) = match s.strip_prefix('-') {
        Some(rest) if rest.chars().any(|c| c.is_ascii_digit() && c != '0') => ("-", rest),
        Some(rest) => ("", rest),
        None => ("", s.as_str()),
    };
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

/// A JSON number with `places` decimals, or `none` when the value is not a number.
pub fn val(v: &Value, places: usize, none: &str) -> String {
    v.as_f64()
        .map_or_else(|| none.to_string(), |x| num(x, places))
}

/// A chainage in metres, printed in kilometres to three places.
pub fn km(metres: f64) -> String {
    num(metres / 1000.0, 3)
}

/// A volume in cubic metres, to the whole number.
pub fn m3(v: &Value) -> String {
    val(v, 0, "not computed")
}

/// A JSON string, or `none`.
pub fn text<'a>(v: &'a Value, none: &'a str) -> &'a str {
    v.as_str().unwrap_or(none)
}

/// Text from the document, made safe to place in Markdown prose: every character Markdown reads
/// as markup is escaped.
pub fn md(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '[' | ']' | '#' | '<' | '>' | '|' | '!'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A longitude and latitude, to five decimals.
pub fn position(v: &Value) -> Option<String> {
    let lon = v.get(0)?.as_f64()?;
    let lat = v.get(1)?.as_f64()?;
    Some(format!("{lon:.5}, {lat:.5}"))
}

#[cfg(test)]
mod tests {
    //! Label: property
    use super::*;

    #[test]
    fn numbers_are_grouped_and_rounded() {
        assert_eq!(num(1234567.891, 0), "1,234,568");
        assert_eq!(num(1234.5, 1), "1,234.5");
        assert_eq!(num(999.0, 0), "999");
        assert_eq!(num(-1234.0, 0), "-1,234");
        assert_eq!(num(-0.0001, 2), "0.00");
        assert_eq!(km(3027.380155), "3.027");
    }

    #[test]
    fn markdown_markup_is_escaped() {
        assert_eq!(md("a_b *c* [d]"), "a\\_b \\*c\\* \\[d\\]");
    }
}
