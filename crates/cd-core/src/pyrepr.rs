//! Python's `repr()` of a `str`, for error messages that quote a user's text with `!r`.

/// `repr(s)` for a Python string: single quotes unless the text holds a single quote and no double
/// quote; backslash, newline, return, tab and control characters escaped.
///
/// Limit: characters that Python calls unprintable but that are not ASCII controls (some
/// separators and format characters) are left as they are. No case in the parity set holds one.
pub fn repr_str(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::with_capacity(s.len() + 2);
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

#[cfg(test)]
mod tests {
    use super::repr_str;

    #[test]
    fn plain_text_gets_single_quotes() {
        assert_eq!(repr_str("v21"), "'v21'");
    }

    #[test]
    fn a_single_quote_switches_to_double_quotes() {
        assert_eq!(repr_str("it's"), "\"it's\"");
    }

    #[test]
    fn both_quotes_keep_single_quotes_and_escape_them() {
        assert_eq!(repr_str("a'b\"c"), "'a\\'b\"c'");
    }

    #[test]
    fn controls_and_backslash_are_escaped() {
        assert_eq!(repr_str("a\\b\nc\t\u{1}"), "'a\\\\b\\nc\\t\\x01'");
    }
}
