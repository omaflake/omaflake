// SPDX-License-Identifier: AGPL-3.0-or-later
//! A Rust format string as a text key (docs/api.md §3, §10.2): every placeholder (`{}`, `{name}`, `{0:?}`, `{x:.1}`)
//! becomes `%@`, `{{`/`}}` become braces and `%` becomes `%%` – the key the translations are looked up by. Shared by
//! the `vm_err!` macro and omaflake-core's tests (which read the macro's format strings out of the sources).

/// What a format string says
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Format {
    /// The key: the literal text with `%@` for each placeholder
    pub key: String,
    /// The placeholders as written (`{}`, `{name:?}`), in order
    pub placeholders: Vec<String>,
    /// Names the string takes from the scope (`{name}`, `{:width$}`), each once, in order
    pub captures: Vec<String>,
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && s != "_"
}

/// Parses a format string (its value, escapes already resolved)
pub fn parse(fmt: &str) -> Result<Format, String> {
    let mut key = String::new();
    let mut placeholders = Vec::new();
    let mut captures: Vec<String> = Vec::new();
    let mut capture = |name: &str| {
        if is_ident(name) && !captures.iter().any(|c| c == name) {
            captures.push(name.to_string());
        }
    };
    let mut chars = fmt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                key.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                key.push('}');
            }
            '{' => {
                let mut inner = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => inner.push(c),
                        None => return Err(format!("unterminated placeholder in {fmt:?}")),
                    }
                }
                let (arg, spec) = match inner.find(':') {
                    Some(i) => (&inner[..i], &inner[i + 1..]),
                    None => (inner.as_str(), ""),
                };
                capture(arg.trim());
                // width and precision may name a value: `{:>width$}`, `{:.prec$}`
                let mut word = String::new();
                for c in spec.chars() {
                    if c.is_alphanumeric() || c == '_' {
                        word.push(c);
                    } else {
                        if c == '$' {
                            capture(&word);
                        }
                        word.clear();
                    }
                }
                placeholders.push(format!("{{{inner}}}"));
                key.push_str("%@");
            }
            '}' => return Err(format!("unmatched }} in {fmt:?}")),
            '%' => key.push_str("%%"),
            c => key.push(c),
        }
    }
    Ok(Format { key, placeholders, captures })
}

/// The value of a string literal token as written in the source (`"a\"b"`, `r#"…"#`)
pub fn unquote(token: &str) -> Result<String, String> {
    if let Some(raw) = token.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        let body = &raw[hashes..];
        let end = body.len().checked_sub(1 + hashes).ok_or("not a string literal")?;
        if !body.starts_with('"') || body.as_bytes()[end] != b'"' {
            return Err(format!("not a string literal: {token}"));
        }
        return Ok(body[1..end].to_string());
    }
    let body = token
        .strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .ok_or_else(|| format!("not a string literal: {token}"))?;
    let mut out = String::new();
    let mut chars = body.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('\'') => out.push('\''),
            Some('"') => out.push('"'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                out.push(u8::from_str_radix(&hex, 16).map_err(|e| e.to_string())? as char);
            }
            Some('u') => {
                let hex: String = chars.by_ref().skip(1).take_while(|c| *c != '}').collect();
                let n = u32::from_str_radix(&hex, 16).map_err(|e| e.to_string())?;
                out.push(char::from_u32(n).ok_or("not a char")?);
            }
            Some('\n') => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            other => return Err(format!("unknown escape \\{other:?}")),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_of_format_strings() {
        let f = parse("Instance \"{}\" does not exist").unwrap();
        assert_eq!(f.key, "Instance \"%@\" does not exist");
        assert_eq!(f.placeholders, ["{}"]);
        let f = parse("{name}: {0:?} – {{x}} at 5% {v:>width$.1}").unwrap();
        assert_eq!(f.key, "%@: %@ – {x} at 5%% %@");
        assert_eq!(f.placeholders, ["{name}", "{0:?}", "{v:>width$.1}"]);
        assert_eq!(f.captures, ["name", "v", "width"]);
        assert!(parse("a } b").is_err());
        assert!(parse("a { b").is_err());
    }

    #[test]
    fn literals() {
        assert_eq!(unquote(r#""a\"b\n\u{2013}\x41""#).unwrap(), "a\"b\n–A");
        assert_eq!(unquote(r##"r#"a"b"#"##).unwrap(), "a\"b");
        assert_eq!(unquote("r\"x\"").unwrap(), "x");
        assert_eq!(unquote("\"one \\\n      two\"").unwrap(), "one two");
    }
}
