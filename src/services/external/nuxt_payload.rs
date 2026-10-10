//! Lenient reader for the Nuxt 2 SSR `window.__NUXT__` payload.
//!
//! Nuxt serialises the page store as a self-invoking function:
//!
//! ```text
//! window.__NUXT__=(function(a,b,…){return {…body…}}(arg1,arg2,…));
//! ```
//!
//! The body is a JavaScript object/array literal. Values that repeat elsewhere
//! in the payload are hoisted into the positional argument list (the body then
//! carries the parameter identifier); values that appear once stay inline.
//! Callers want the body as a [`serde_json::Value`] with identifiers resolved
//! against the argument list.
//!
//! The parser is deliberately total: any structural surprise yields `None`,
//! never an error or a panic. Only the JavaScript subset Nuxt emits is
//! supported (no operators, function bodies or template strings); the
//! defensive `!0` / `!1` / `void 0` / `undefined` spellings are accepted too.

use std::collections::HashMap;

use serde_json::{Map, Number, Value};

const MARKER: &str = "window.__NUXT__=(function(";

/// Nesting cap for the recursive descent: a hostile page must not overflow the
/// stack through a 400 KB payload of `[[[[…`. Real Nuxt bodies stay far below.
const MAX_DEPTH: usize = 128;

/// Extract the payload body of the first `window.__NUXT__` IIFE in `html`.
/// `None` when the marker is missing or the payload cannot be parsed.
pub fn extract(html: &str) -> Option<Value> {
    let start = html.find(MARKER)? + MARKER.len();
    let mut p = Parser {
        src: html,
        pos: start,
        depth: 0,
    };

    // `function(a,b,…)` — positional parameter names.
    let mut params: Vec<String> = Vec::new();
    loop {
        p.skip_ws();
        if p.eat(")") {
            break;
        }
        if !params.is_empty() {
            p.expect(",")?;
            p.skip_ws();
            if p.eat(")") {
                break;
            }
        }
        params.push(p.ident()?);
    }

    p.skip_ws();
    p.expect("{")?;
    p.skip_ws();
    p.expect("return")?;
    let body = p.value()?;

    // `}(arg1,arg2,…))` — the argument list holds primitives only.
    p.skip_ws();
    p.expect("}(")?;
    let mut args: Vec<Value> = Vec::new();
    loop {
        p.skip_ws();
        if p.eat(")") {
            break;
        }
        if !args.is_empty() {
            p.expect(",")?;
            p.skip_ws();
            if p.eat(")") {
                break;
            }
        }
        match p.value()? {
            Node::Value(v) => args.push(v),
            _ => return None,
        }
    }

    let index: HashMap<&str, usize> = params
        .iter()
        .enumerate()
        .map(|(i, name)| (name.as_str(), i))
        .collect();
    resolve(&body, &index, &args)
}

/// Parsed body before identifier resolution.
enum Node {
    Value(Value),
    Ident(String),
    Array(Vec<Node>),
    Object(Vec<(String, Node)>),
}

/// Total bytes one resolution may produce. Hoisted arguments can be referenced
/// many times, so a crafted payload could expand to gigabytes; the budget turns
/// that into a plain `None`.
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

/// Replace every identifier with its positional argument; fail on a reference
/// the parameter list does not define or when the expansion exceeds
/// [`MAX_OUTPUT_BYTES`].
fn resolve(node: &Node, index: &HashMap<&str, usize>, args: &[Value]) -> Option<Value> {
    let mut budget = MAX_OUTPUT_BYTES;
    resolve_node(node, index, args, &mut budget)
}

fn resolve_node(
    node: &Node,
    index: &HashMap<&str, usize>,
    args: &[Value],
    budget: &mut usize,
) -> Option<Value> {
    match node {
        Node::Value(v) => {
            if charge(v, budget) {
                Some(v.clone())
            } else {
                None
            }
        }
        Node::Ident(name) => {
            let v = args.get(*index.get(name.as_str())?)?;
            if charge(v, budget) {
                Some(v.clone())
            } else {
                None
            }
        }
        Node::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(resolve_node(item, index, args, budget)?);
            }
            Some(Value::Array(out))
        }
        Node::Object(fields) => {
            let mut map = Map::new();
            for (key, value) in fields {
                map.insert(key.clone(), resolve_node(value, index, args, budget)?);
            }
            Some(Value::Object(map))
        }
    }
}

/// Charge one value's size against the budget; `false` when it no longer fits.
fn charge(value: &Value, budget: &mut usize) -> bool {
    let size = value_size(value);
    if size > *budget {
        return false;
    }
    *budget -= size;
    true
}

fn value_size(value: &Value) -> usize {
    match value {
        Value::String(s) => s.len(),
        Value::Array(items) => items.iter().map(value_size).sum(),
        Value::Object(map) => map.values().map(value_size).sum(),
        _ => 8,
    }
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    fn rest(&self) -> &str {
        &self.src[self.pos..]
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn eat(&mut self, lit: &str) -> bool {
        if self.rest().starts_with(lit) {
            self.pos += lit.len();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, lit: &str) -> Option<()> {
        self.eat(lit).then_some(())
    }

    /// Keyword literal that must not be a prefix of a longer identifier.
    fn keyword(&mut self, lit: &str) -> bool {
        let Some(after) = self.rest().strip_prefix(lit) else {
            return false;
        };
        if after.chars().next().is_some_and(is_ident_continue) {
            return false;
        }
        self.pos += lit.len();
        true
    }

    fn ident(&mut self) -> Option<String> {
        let rest = self.rest();
        let first = rest.chars().next()?;
        if !is_ident_start(first) {
            return None;
        }
        let mut end = first.len_utf8();
        for c in rest[end..].chars() {
            if is_ident_continue(c) {
                end += c.len_utf8();
            } else {
                break;
            }
        }
        let name = rest[..end].to_string();
        self.pos += end;
        Some(name)
    }

    fn value(&mut self) -> Option<Node> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        self.depth += 1;
        let parsed = self.value_inner();
        self.depth -= 1;
        parsed
    }

    fn value_inner(&mut self) -> Option<Node> {
        self.skip_ws();
        let c = self.peek()?;
        match c {
            '{' => return self.object(),
            '[' => return self.array(),
            '"' | '\'' => return self.string().map(|s| Node::Value(Value::String(s))),
            _ => {}
        }
        if self.keyword("true") {
            return Some(Node::Value(Value::Bool(true)));
        }
        if self.keyword("false") {
            return Some(Node::Value(Value::Bool(false)));
        }
        if self.keyword("null") || self.keyword("undefined") {
            return Some(Node::Value(Value::Null));
        }
        if self.keyword("void") {
            self.skip_ws();
            return self.expect("0").map(|_| Node::Value(Value::Null));
        }
        if self.eat("!0") {
            return Some(Node::Value(Value::Bool(true)));
        }
        if self.eat("!1") {
            return Some(Node::Value(Value::Bool(false)));
        }
        if c == '-' || c.is_ascii_digit() {
            return self.number().map(Node::Value);
        }
        if is_ident_start(c) {
            return self.ident().map(Node::Ident);
        }
        None
    }

    fn object(&mut self) -> Option<Node> {
        self.expect("{")?;
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            if self.eat("}") {
                return Some(Node::Object(fields));
            }
            if !fields.is_empty() {
                self.expect(",")?;
                self.skip_ws();
                if self.eat("}") {
                    return Some(Node::Object(fields));
                }
            }
            let key = match self.peek()? {
                '"' | '\'' => self.string()?,
                c if is_ident_start(c) => self.ident()?,
                // Numeric key (a Vuex map keyed by index): take the literal.
                c if c.is_ascii_digit() || c == '-' => {
                    let rest = self.rest();
                    let end = rest
                        .char_indices()
                        .skip(1)
                        .find(|(_, c)| !c.is_ascii_digit())
                        .map(|(i, _)| i)
                        .unwrap_or(rest.len());
                    let key = rest[..end].to_string();
                    self.pos += end;
                    key
                }
                _ => return None,
            };
            self.skip_ws();
            self.expect(":")?;
            let value = self.value()?;
            fields.push((key, value));
        }
    }

    fn array(&mut self) -> Option<Node> {
        self.expect("[")?;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.eat("]") {
                return Some(Node::Array(items));
            }
            if !items.is_empty() {
                self.expect(",")?;
                self.skip_ws();
                if self.eat("]") {
                    return Some(Node::Array(items));
                }
            }
            items.push(self.value()?);
        }
    }

    fn number(&mut self) -> Option<Value> {
        let rest = self.rest();
        let bytes = rest.as_bytes();
        let mut i = 0usize;
        if bytes.first() == Some(&b'-') {
            i += 1;
        }
        let int_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i == int_start {
            return None;
        }
        let mut float = false;
        if i < bytes.len() && bytes[i] == b'.' {
            float = true;
            i += 1;
            let frac_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == frac_start {
                return None;
            }
        }
        if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            float = true;
            i += 1;
            if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                i += 1;
            }
            let exp_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i == exp_start {
                return None;
            }
        }
        let text = &rest[..i];
        let value = if float {
            text.parse::<f64>()
                .ok()
                .and_then(Number::from_f64)
                .map(Value::Number)
        } else {
            text.parse::<i64>()
                .map(Number::from)
                .or_else(|_| {
                    text.parse::<f64>()
                        .ok()
                        .and_then(Number::from_f64)
                        .ok_or(())
                })
                .map(Value::Number)
                .ok()
        };
        self.pos += i;
        value
    }

    fn string(&mut self) -> Option<String> {
        let quote = self.peek()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        self.pos += quote.len_utf8();
        let mut out = String::new();
        loop {
            let c = self.peek()?;
            if c == quote {
                self.pos += c.len_utf8();
                return Some(out);
            }
            if c != '\\' {
                out.push(c);
                self.pos += c.len_utf8();
                continue;
            }
            self.pos += 1;
            match self.peek()? {
                '"' => {
                    out.push('"');
                    self.pos += 1;
                }
                '\'' => {
                    out.push('\'');
                    self.pos += 1;
                }
                '\\' => {
                    out.push('\\');
                    self.pos += 1;
                }
                '/' => {
                    out.push('/');
                    self.pos += 1;
                }
                'n' => {
                    out.push('\n');
                    self.pos += 1;
                }
                'r' => {
                    out.push('\r');
                    self.pos += 1;
                }
                't' => {
                    out.push('\t');
                    self.pos += 1;
                }
                'b' => {
                    out.push('\u{0008}');
                    self.pos += 1;
                }
                'f' => {
                    out.push('\u{000C}');
                    self.pos += 1;
                }
                'u' => {
                    self.pos += 1;
                    out.push(self.unicode_escape()?);
                }
                '\n' => {
                    self.pos += 1; // line continuation
                }
                other => {
                    out.push(other);
                    self.pos += other.len_utf8();
                }
            }
        }
    }

    /// One `\uXXXX` escape (the `\u` is already consumed). Surrogate pairs
    /// combine; an unpaired surrogate degrades to U+FFFD.
    fn unicode_escape(&mut self) -> Option<char> {
        let first = self.hex4()?;
        if (0xD800..=0xDBFF).contains(&first) {
            let save = self.pos;
            if self.eat("\\u")
                && let Some(second) = self.hex4().filter(|s| (0xDC00..=0xDFFF).contains(s))
            {
                let combined = 0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00);
                return Some(char::from_u32(combined).unwrap_or('\u{FFFD}'));
            }
            self.pos = save;
            return Some('\u{FFFD}');
        }
        Some(char::from_u32(first).unwrap_or('\u{FFFD}'))
    }

    fn hex4(&mut self) -> Option<u32> {
        let hex = self.src.get(self.pos..self.pos + 4)?;
        if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        self.pos += 4;
        u32::from_str_radix(hex, 16).ok()
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

fn is_ident_continue(c: char) -> bool {
    is_ident_start(c) || c.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_script(payload: &str) -> Option<Value> {
        extract(&format!("<html><script>{payload}</script></body></html>"))
    }

    #[test]
    fn resolves_params_and_inline_values() {
        let v = extract_script(
            r#"window.__NUXT__=(function(a,b,c){return {x:a,y:{z:b},list:[c,{k:"v"},1,2.5,true,null]}}(false,42,"\u002Fok"));"#,
        )
        .expect("payload must parse");
        assert_eq!(v.pointer("/x"), Some(&Value::Bool(false)));
        assert_eq!(v.pointer("/y/z"), Some(&Value::from(42)));
        // `\u002F` decodes to `/`.
        assert_eq!(v.pointer("/list/0"), Some(&Value::from("/ok")));
        assert_eq!(v.pointer("/list/1/k"), Some(&Value::from("v")));
        assert_eq!(v.pointer("/list/2"), Some(&Value::from(1)));
        assert_eq!(v.pointer("/list/3"), Some(&Value::from(2.5)));
        assert_eq!(v.pointer("/list/4"), Some(&Value::Bool(true)));
        assert_eq!(v.pointer("/list/5"), Some(&Value::Null));
    }

    #[test]
    fn defensive_spellings() {
        let v = extract_script(
            "window.__NUXT__=(function(a,b,c,d,e,f,g){return [a,b,c,d,e,f,g]}(!0,!1,void 0,undefined,true,false,null));",
        )
        .expect("payload must parse");
        assert_eq!(
            v,
            serde_json::json!([true, false, null, null, true, false, null])
        );
    }

    #[test]
    fn single_quotes_trailing_commas_and_surrogates() {
        let v = extract_script(
            "window.__NUXT__=(function(x){return {s:'a\\'b',e:'\\uD83D\\uDE00',arr:[x,]}}(7,));",
        )
        .expect("payload must parse");
        assert_eq!(v.pointer("/s"), Some(&Value::from("a'b")));
        assert_eq!(v.pointer("/e"), Some(&Value::from("\u{1F600}")));
        assert_eq!(v.pointer("/arr/0"), Some(&Value::from(7)));
    }

    #[test]
    fn keyword_params_are_not_misread() {
        // A parameter literally named `n` (or `t`, `u`, `f`) must resolve as an
        // identifier, not be eaten as a keyword prefix.
        let v = extract_script(
            "window.__NUXT__=(function(n,t,u,f){return {n:n,t:t,u:u,f:f}}(1,2,3,4));",
        )
        .expect("payload must parse");
        assert_eq!(v.pointer("/n"), Some(&Value::from(1)));
        assert_eq!(v.pointer("/t"), Some(&Value::from(2)));
        assert_eq!(v.pointer("/u"), Some(&Value::from(3)));
        assert_eq!(v.pointer("/f"), Some(&Value::from(4)));
    }

    #[test]
    fn deep_nesting_is_rejected_not_overflowed() {
        let depth = MAX_DEPTH + 10;
        let payload = format!(
            "window.__NUXT__=(function(a){{return {}{}{}}}(1));",
            "[".repeat(depth),
            "a",
            "]".repeat(depth)
        );
        assert!(extract_script(&payload).is_none());
        // A shallow nesting still parses.
        let ok = format!(
            "window.__NUXT__=(function(a){{return {}a{}}}(1));",
            "[".repeat(10),
            "]".repeat(10)
        );
        assert!(extract_script(&ok).is_some());
    }

    #[test]
    fn output_budget_blocks_amplification() {
        // One 1 MB argument referenced 20 times would expand to ~20 MB without
        // the budget (MAX_OUTPUT_BYTES is 16 MB).
        let big = "x".repeat(1024 * 1024);
        let body = format!("{}a{}", "[a,".repeat(20), "]".repeat(20));
        let payload = format!("window.__NUXT__=(function(a){{return {body}}}(\"{big}\"));");
        assert!(extract_script(&payload).is_none());
        // A small payload is unaffected.
        let ok = "window.__NUXT__=(function(a){return [a,a,a]}(\"tiny\"));";
        assert!(extract_script(ok).is_some());
    }

    #[test]
    fn numeric_object_keys_parse() {
        let v = extract_script("window.__NUXT__=(function(a){return {1:a,2:\"b\",-3:true}}(7));")
            .expect("payload must parse");
        assert_eq!(v.pointer("/1"), Some(&Value::from(7)));
        assert_eq!(v.pointer("/2"), Some(&Value::from("b")));
        assert_eq!(v.pointer("/-3"), Some(&Value::Bool(true)));
    }

    #[test]
    fn malformed_payloads_are_none() {
        assert!(extract("no marker here").is_none());
        assert!(extract_script("window.__NUXT__=(function(a){return {x:a}}(;").is_none());
        // An argument list holding a non-primitive is rejected.
        assert!(extract_script("window.__NUXT__=(function(a){return {x:a}}({y:1}));").is_none());
        // An identifier without a matching parameter is rejected.
        assert!(extract_script("window.__NUXT__=(function(a){return {x:b}}(1));").is_none());
        // A broken string is rejected.
        assert!(extract_script("window.__NUXT__=(function(a){return {x:\"oops}}(1));").is_none());
    }

    #[test]
    fn parses_kuaikan_fixture() {
        let html = include_str!("../../../tests/fixtures/kuaikan_detail.html");
        let v = extract(html).expect("kuaikan fixture must parse");
        assert_eq!(
            v.pointer("/data/0/topicInfo/title").and_then(Value::as_str),
            Some("妖神记")
        );
        let list = v
            .pointer("/data/0/comicList")
            .and_then(Value::as_array)
            .expect("comicList array");
        assert_eq!(list.len(), 927);
        // Served oldest-first: the first row is chapter 1.
        assert_eq!(
            list[0].get("title").and_then(Value::as_str),
            Some("第1话 重生")
        );
        // Inline `created_at` epoch-ms survives the round trip.
        assert!(
            list[0]
                .get("created_at")
                .and_then(Value::as_i64)
                .is_some_and(|ms| ms > 1_500_000_000_000)
        );
    }
}
