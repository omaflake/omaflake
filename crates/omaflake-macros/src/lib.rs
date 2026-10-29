// SPDX-License-Identifier: AGPL-3.0-or-later
//! The macro behind omaflake-core's `vm_err!` (plan B125): an error whose message is a format string keeps that
//! string as its text key and the values it was filled with as the key's args (docs/api.md §3), so every interface
//! can show it in its language – without touching the call sites.
//!
//! `vm_err!("Instance \"{}\" does not exist", name)` and `vm_err!(codes::NOT_FOUND, "No such folder: {path}")` expand
//! (through omaflake-core's `macro_rules!` wrapper, which passes `$crate` first) to
//!
//! ```text
//! $crate::error::Error::__keyed(None, "Instance \"%@\" does not exist", 1, format!("{}", name))
//! ```
//!
//! – one `format!` with the placeholders as written, joined by a separator, so each value is formatted once and
//! exactly as the message would have it. Names the string takes from the scope (`{path}`) become named args with the
//! literal's span, so they resolve where the macro was called.

#![forbid(unsafe_code)]

use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

mod key;

/// Between the formatted values in the joined string (omaflake-core splits by it)
const SEPARATOR: &str = "\u{1f}\u{1e}\u{1f}";

fn error(message: &str, span: Span) -> TokenStream {
    let mut lit = Literal::string(message);
    lit.set_span(span);
    let mut out: TokenStream = "::core::compile_error!".parse().unwrap();
    out.extend([TokenTree::Group(Group::new(Delimiter::Parenthesis, TokenTree::Literal(lit).into()))]);
    out
}

/// A token, out of the invisible groups macro_rules fragments come in
fn unwrap_none(t: TokenTree) -> TokenTree {
    match t {
        TokenTree::Group(g) if g.delimiter() == Delimiter::None => {
            let mut inner = g.stream().into_iter();
            match (inner.next(), inner.next()) {
                (Some(one), None) => unwrap_none(one),
                _ => TokenTree::Group(g),
            }
        }
        t => t,
    }
}

fn is_punct(t: &TokenTree, c: char) -> bool {
    matches!(t, TokenTree::Punct(p) if p.as_char() == c)
}

/// `(krate) [code,] "format", args…`
#[doc(hidden)]
#[proc_macro]
pub fn keyed_error(input: TokenStream) -> TokenStream {
    let mut tokens = input.into_iter();
    let krate = match tokens.next() {
        Some(TokenTree::Group(g)) => g.stream(),
        _ => return error("vm_err!: called without its crate", Span::call_site()),
    };
    let rest: Vec<TokenTree> = tokens.collect();
    // A code first (anything but the literal): up to the first comma
    let starts_with_literal =
        rest.first().cloned().map(unwrap_none).is_some_and(|t| matches!(t, TokenTree::Literal(_)));
    let (code, rest) = match rest.iter().position(|t| is_punct(t, ',')) {
        Some(i) if !starts_with_literal => (Some(rest[..i].to_vec()), rest[i + 1..].to_vec()),
        _ => (None, rest),
    };
    let mut rest = rest.into_iter();
    let lit = match rest.next().map(unwrap_none) {
        Some(TokenTree::Literal(l)) => l,
        Some(other) => return error("vm_err!: a format string literal comes first", other.span()),
        None => return error("vm_err!: a format string is missing", Span::call_site()),
    };
    let span = lit.span();
    let value = match key::unquote(&lit.to_string()) {
        Ok(v) => v,
        Err(e) => return error(&format!("vm_err!: {e}"), span),
    };
    let format = match key::parse(&value) {
        Ok(f) => f,
        Err(e) => return error(&format!("vm_err!: {e}"), span),
    };
    let mut args: Vec<TokenTree> = rest.collect();
    if args.last().is_some_and(|t| is_punct(t, ',')) {
        args.pop();
    }
    // Named args given (`name = expr`): those names are not taken from the scope
    let mut named = Vec::new();
    for item in args.split(|t| is_punct(t, ',')) {
        if let [TokenTree::Ident(name), TokenTree::Punct(eq), next, ..] = item
            && eq.as_char() == '='
            && eq.spacing() == Spacing::Alone
            && !is_punct(next, '=')
        {
            named.push(name.to_string());
        }
    }

    // format!(joined, args…, capture = capture…)
    let mut joined = Literal::string(&format.placeholders.join(SEPARATOR));
    joined.set_span(span);
    let mut format_args: Vec<TokenTree> = vec![TokenTree::Literal(joined)];
    format_args.extend(args.iter().cloned());
    for name in format.captures.iter().filter(|n| !named.contains(n)) {
        format_args.push(TokenTree::Punct(Punct::new(',', Spacing::Alone)));
        format_args.push(TokenTree::Ident(Ident::new(name, span)));
        format_args.push(TokenTree::Punct(Punct::new('=', Spacing::Alone)));
        format_args.push(TokenTree::Ident(Ident::new(name, span)));
    }
    let values: TokenStream = if format.placeholders.is_empty() && args.is_empty() {
        "::std::string::String::new()".parse().unwrap()
    } else {
        let mut t: TokenStream = "::std::format!".parse().unwrap();
        t.extend([TokenTree::Group(Group::new(Delimiter::Parenthesis, format_args.into_iter().collect()))]);
        t
    };

    let code: TokenStream = match code {
        Some(code) => {
            let mut t: TokenStream = "::core::option::Option::Some".parse().unwrap();
            t.extend([TokenTree::Group(Group::new(Delimiter::Parenthesis, code.into_iter().collect()))]);
            t
        }
        None => "::core::option::Option::None".parse().unwrap(),
    };
    let mut call_args = code;
    call_args.extend([TokenTree::Punct(Punct::new(',', Spacing::Alone))]);
    let mut key = Literal::string(&format.key);
    key.set_span(span);
    call_args.extend([TokenTree::Literal(key), TokenTree::Punct(Punct::new(',', Spacing::Alone))]);
    call_args.extend([TokenTree::Literal(Literal::usize_suffixed(format.placeholders.len()))]);
    call_args.extend([TokenTree::Punct(Punct::new(',', Spacing::Alone))]);
    call_args.extend(values);

    let mut out = krate;
    out.extend("::error::Error::__keyed".parse::<TokenStream>().unwrap());
    out.extend([TokenTree::Group(Group::new(Delimiter::Parenthesis, call_args))]);
    out
}
