//! ISO 10303-21 ("STEP physical file", `.ifc`/`.stp`) reader.
//!
//! Generic: it knows the exchange-file grammar, not any EXPRESS schema, so it
//! reads IFC2X3, IFC4 and IFC4X3 alike and leaves the meaning of attribute
//! positions to the caller (see `ids.rs`). Part of issue #66 (IFC 4.3
//! inlezen).
//!
//! Grammar per ISO 10303-21:2016 §5–§8: a `HEADER;` section with entities
//! without instance names (`FILE_SCHEMA(('IFC4X3_ADD2'));`), one or more
//! `DATA;` sections with `#id = NAME(args);` instances, `ENDSEC;` and
//! `END-ISO-10303-21;`. Arguments: integers, reals (`1.E-05`, `-0.5`),
//! strings (`'it''s'`, with the §6.4.3 control directives `\X2\…\X0\`
//! (UTF-16), `\X4\…\X0\` (UTF-32), `\X\hh` and `\S\c` (ISO 8859-1), `\\`;
//! `\P?\` code-page switches are dropped), enumerations (`.T.`), instance
//! references (`#12`), unset `$`, derived `*`, nested lists `(…)`, typed
//! parameters (`IFCLABEL('x')`) and binaries (`"0FF"`). Comments `/* */`
//! may appear between any two tokens. Complex instances `#1=(A(…)B(…));`
//! are flattened: name = the partial types joined by `+`, arguments
//! concatenated in order.
//!
//! The parser works on bytes in one pass without regular expressions: a
//! synthetic 50 MB file (650 000 instances) reads in about 1.6 s in a
//! release build (test `step_speed`, ignored by default). Raw (non-escaped)
//! UTF-8 inside strings, which many exporters write, is accepted.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

/// One argument value of a STEP instance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// `$`: no value.
    Null,
    /// `*`: value derived from other attributes.
    Derived,
    Int(i64),
    Real(f64),
    /// String with escapes decoded.
    Str(String),
    /// Enumeration without the dots, e.g. `T` for `.T.`.
    Enum(String),
    /// Instance reference `#id`.
    Ref(u64),
    List(Vec<Value>),
    /// Typed parameter such as `IFCLABEL('x')` (type name uppercase).
    Typed(String, Box<Value>),
    /// Binary `"…"` as its hex text.
    Binary(String),
}

impl Value {
    /// Unwraps typed parameters (`IFCLABEL('x')` → `'x'`).
    pub fn inner(&self) -> &Value {
        match self {
            Value::Typed(_, v) => v.inner(),
            v => v,
        }
    }
    /// The referenced instance id.
    pub fn as_ref(&self) -> Option<u64> {
        match self.inner() {
            Value::Ref(r) => Some(*r),
            _ => None,
        }
    }
    /// The decoded string (through typed parameters).
    pub fn as_str(&self) -> Option<&str> {
        match self.inner() {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    /// Integer or real as `f64` (through typed parameters).
    pub fn as_f64(&self) -> Option<f64> {
        match self.inner() {
            Value::Int(i) => Some(*i as f64),
            Value::Real(r) => Some(*r),
            _ => None,
        }
    }
    /// Integer value (through typed parameters).
    pub fn as_i64(&self) -> Option<i64> {
        match self.inner() {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    /// Enumeration literal without dots.
    pub fn as_enum(&self) -> Option<&str> {
        match self.inner() {
            Value::Enum(e) => Some(e),
            _ => None,
        }
    }
    /// `.T.`/`.F.` as a boolean; `.U.` (logical unknown) gives `None`.
    pub fn as_bool(&self) -> Option<bool> {
        match self.as_enum()? {
            "T" => Some(true),
            "F" => Some(false),
            _ => None,
        }
    }
    /// List elements (through typed parameters).
    pub fn as_list(&self) -> Option<&[Value]> {
        match self.inner() {
            Value::List(l) => Some(l),
            _ => None,
        }
    }
    /// The references in a list (or the single reference) in order.
    pub fn refs(&self) -> Vec<u64> {
        let mut out = Vec::new();
        self.collect_refs(&mut out);
        out
    }
    fn collect_refs(&self, out: &mut Vec<u64>) {
        match self {
            Value::Ref(r) => out.push(*r),
            Value::List(l) => l.iter().for_each(|v| v.collect_refs(out)),
            Value::Typed(_, v) => v.collect_refs(out),
            _ => {}
        }
    }
    /// `$` or `*`.
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null | Value::Derived)
    }
    /// The type name of a typed parameter (`IFCLABEL`).
    pub fn type_name(&self) -> Option<&str> {
        match self {
            Value::Typed(t, _) => Some(t),
            _ => None,
        }
    }
}

/// One instance `#id = NAME(args);`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub id: u64,
    /// Type name, uppercase (`IFCWALL`).
    pub name: String,
    pub args: Vec<Value>,
}

impl Entity {
    /// Argument `i` (0-based), `None` past the end.
    pub fn arg(&self, i: usize) -> Option<&Value> {
        self.args.get(i)
    }
    /// Argument `i` as a reference.
    pub fn ref_at(&self, i: usize) -> Option<u64> {
        self.arg(i)?.as_ref()
    }
    /// Argument `i` as a string.
    pub fn str_at(&self, i: usize) -> Option<&str> {
        self.arg(i)?.as_str()
    }
    /// Argument `i` as a number.
    pub fn f64_at(&self, i: usize) -> Option<f64> {
        self.arg(i)?.as_f64()
    }
    /// Argument `i` as an enumeration literal.
    pub fn enum_at(&self, i: usize) -> Option<&str> {
        self.arg(i)?.as_enum()
    }
    /// Argument `i` as a list.
    pub fn list_at(&self, i: usize) -> Option<&[Value]> {
        self.arg(i)?.as_list()
    }
    /// The references in argument `i` (list or single).
    pub fn refs_at(&self, i: usize) -> Vec<u64> {
        self.arg(i).map(Value::refs).unwrap_or_default()
    }
}

/// The HEADER section.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// FILE_SCHEMA identifiers (`IFC4X3_ADD2`).
    pub schema: Vec<String>,
    /// FILE_DESCRIPTION descriptions.
    pub description: Vec<String>,
    /// FILE_NAME name.
    pub file_name: Option<String>,
    /// All header entities as read.
    pub entities: Vec<(String, Vec<Value>)>,
}

/// A parsed exchange file with lookup by id and by type.
#[derive(Debug, Default)]
pub struct StepModel {
    pub header: Header,
    entities: Vec<Entity>,
    index: HashMap<u64, usize>,
    by_type: HashMap<String, Vec<usize>>,
    inverse: OnceLock<HashMap<u64, Vec<u64>>>,
}

impl StepModel {
    /// Parses a STEP physical file.
    pub fn parse(bytes: &[u8]) -> Result<StepModel, String> {
        let mut p = Parser { b: bytes, i: 0 };
        let mut header = Header::default();
        let mut entities = Vec::new();
        p.skip_ws();
        if !p.eat_keyword(b"ISO-10303-21") {
            return Err("not a STEP file: missing ISO-10303-21".into());
        }
        p.expect(b';')?;
        loop {
            p.skip_ws();
            if p.i >= p.b.len() {
                break;
            }
            if p.eat_keyword(b"END-ISO-10303-21") {
                break;
            }
            let kw = p.keyword()?;
            match kw.as_str() {
                "HEADER" => {
                    p.expect(b';')?;
                    loop {
                        p.skip_ws();
                        let name = p.keyword()?;
                        if name == "ENDSEC" {
                            p.expect(b';')?;
                            break;
                        }
                        p.skip_ws();
                        let args = p.arg_list()?;
                        p.expect(b';')?;
                        let strings = |v: Option<&Value>| -> Vec<String> {
                            v.and_then(Value::as_list)
                                .map(|l| {
                                    l.iter()
                                        .filter_map(|x| x.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default()
                        };
                        match name.as_str() {
                            "FILE_SCHEMA" => header.schema = strings(args.first()),
                            "FILE_DESCRIPTION" => header.description = strings(args.first()),
                            "FILE_NAME" => {
                                header.file_name =
                                    args.first().and_then(|v| v.as_str()).map(String::from)
                            }
                            _ => {}
                        }
                        header.entities.push((name, args));
                    }
                }
                "DATA" => {
                    p.skip_ws();
                    if p.peek() == Some(b'(') {
                        p.arg_list()?; // edition 3: DATA('name',('schema'))
                    }
                    p.expect(b';')?;
                    loop {
                        p.skip_ws();
                        match p.peek() {
                            Some(b'#') => entities.push(p.instance()?),
                            Some(_) => {
                                let k = p.keyword()?;
                                if k != "ENDSEC" {
                                    return Err(
                                        p.err(&format!("expected instance or ENDSEC, found {k}"))
                                    );
                                }
                                p.expect(b';')?;
                                break;
                            }
                            None => return Err("unexpected end of file in DATA section".into()),
                        }
                    }
                }
                "ANCHOR" | "REFERENCE" | "SIGNATURE" => {
                    // Edition 3 sections: skip to ENDSEC.
                    loop {
                        p.skip_ws();
                        if p.eat_keyword(b"ENDSEC") {
                            p.expect(b';')?;
                            break;
                        }
                        if p.i >= p.b.len() {
                            return Err("unexpected end of file".into());
                        }
                        p.skip_token();
                    }
                }
                other => return Err(p.err(&format!("unexpected section {other}"))),
            }
        }
        Ok(StepModel::from_parts(header, entities))
    }

    /// Parses from text.
    pub fn parse_str(text: &str) -> Result<StepModel, String> {
        Self::parse(text.as_bytes())
    }

    /// Builds a model from already parsed instances.
    pub fn from_parts(header: Header, entities: Vec<Entity>) -> StepModel {
        let mut index = HashMap::with_capacity(entities.len());
        let mut by_type: HashMap<String, Vec<usize>> = HashMap::new();
        for (k, e) in entities.iter().enumerate() {
            index.insert(e.id, k);
            match by_type.get_mut(&e.name) {
                Some(v) => v.push(k),
                None => {
                    by_type.insert(e.name.clone(), vec![k]);
                }
            }
        }
        StepModel {
            header,
            entities,
            index,
            by_type,
            inverse: OnceLock::new(),
        }
    }

    /// The first FILE_SCHEMA identifier, uppercase.
    pub fn schema(&self) -> Option<String> {
        self.header.schema.first().map(|s| s.to_ascii_uppercase())
    }
    /// Instance by id.
    pub fn get(&self, id: u64) -> Option<&Entity> {
        self.index.get(&id).map(|&k| &self.entities[k])
    }
    /// Follows a reference value.
    pub fn deref(&self, v: &Value) -> Option<&Entity> {
        self.get(v.as_ref()?)
    }
    /// All instances in file order.
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }
    /// Instances of exactly this type (case-insensitive; no subtypes).
    pub fn by_type(&self, name: &str) -> Vec<&Entity> {
        self.by_type
            .get(&name.to_ascii_uppercase())
            .map(|v| v.iter().map(|&k| &self.entities[k]).collect())
            .unwrap_or_default()
    }
    /// The type names present with their counts, sorted by name.
    pub fn type_counts(&self) -> Vec<(String, usize)> {
        let mut v: Vec<_> = self
            .by_type
            .iter()
            .map(|(k, v)| (k.clone(), v.len()))
            .collect();
        v.sort();
        v
    }
    /// Number of instances.
    pub fn len(&self) -> usize {
        self.entities.len()
    }
    /// True without instances.
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
    /// Ids of all instances that reference `id` in any argument (built once).
    pub fn referencing(&self, id: u64) -> &[u64] {
        let inv = self.inverse.get_or_init(|| {
            let mut m: HashMap<u64, Vec<u64>> = HashMap::new();
            for e in &self.entities {
                let mut refs = Vec::new();
                for a in &e.args {
                    a.collect_refs(&mut refs);
                }
                refs.sort_unstable();
                refs.dedup();
                for r in refs {
                    m.entry(r).or_default().push(e.id);
                }
            }
            m
        });
        inv.get(&id).map(Vec::as_slice).unwrap_or(&[])
    }
    /// Instances of type `name` that reference `id`.
    pub fn referencing_of_type(&self, id: u64, name: &str) -> Vec<&Entity> {
        let name = name.to_ascii_uppercase();
        self.referencing(id)
            .iter()
            .filter_map(|&r| self.get(r))
            .filter(|e| e.name == name)
            .collect()
    }
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, msg: &str) -> String {
        let line = 1 + self.b[..self.i.min(self.b.len())]
            .iter()
            .filter(|&&c| c == b'\n')
            .count();
        format!("STEP line {line}: {msg}")
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    /// Skips whitespace and `/* */` comments.
    fn skip_ws(&mut self) {
        while self.i < self.b.len() {
            let c = self.b[self.i];
            if c.is_ascii_whitespace() {
                self.i += 1;
            } else if c == b'/' && self.b.get(self.i + 1) == Some(&b'*') {
                match find(&self.b[self.i + 2..], b"*/") {
                    Some(k) => self.i += k + 4,
                    None => self.i = self.b.len(),
                }
            } else {
                break;
            }
        }
    }
    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected '{}'", c as char)))
        }
    }
    fn eat_keyword(&mut self, kw: &[u8]) -> bool {
        if self.b[self.i..].starts_with(kw) {
            let next = self.b.get(self.i + kw.len()).copied();
            if !next.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-') {
                self.i += kw.len();
                return true;
            }
        }
        false
    }
    /// A keyword (`IFCWALL`, `FILE_NAME`, `!USER_KW`) uppercased.
    fn keyword(&mut self) -> Result<String, String> {
        self.skip_ws();
        let s = self.i;
        if self.peek() == Some(b'!') {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric() || c == b'_' || c == b'-' {
                self.i += 1;
            } else {
                break;
            }
        }
        if self.i == s {
            return Err(self.err("expected keyword"));
        }
        // Only ASCII bytes were consumed, so this is valid UTF-8.
        let mut k: Vec<u8> = self.b[s..self.i].to_vec();
        k.make_ascii_uppercase();
        Ok(String::from_utf8(k).unwrap_or_default())
    }
    fn skip_token(&mut self) {
        match self.peek() {
            Some(b'\'') => {
                let _ = self.string();
            }
            _ => self.i += 1,
        }
    }
    /// `#id = NAME(args);` or `#id = (A(args)B(args));`.
    fn instance(&mut self) -> Result<Entity, String> {
        self.i += 1; // '#'
        let id = self
            .uint()
            .ok_or_else(|| self.err("expected instance id"))?;
        self.expect(b'=')?;
        self.skip_ws();
        let (name, args) = if self.peek() == Some(b'(') {
            self.i += 1;
            let mut names = Vec::new();
            let mut args = Vec::new();
            loop {
                self.skip_ws();
                if self.peek() == Some(b')') {
                    self.i += 1;
                    break;
                }
                names.push(self.keyword()?);
                self.skip_ws();
                args.extend(self.arg_list()?);
            }
            (names.join("+"), args)
        } else {
            let name = self.keyword()?;
            self.skip_ws();
            (name, self.arg_list()?)
        };
        self.expect(b';')?;
        Ok(Entity { id, name, args })
    }
    fn uint(&mut self) -> Option<u64> {
        let s = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        std::str::from_utf8(&self.b[s..self.i]).ok()?.parse().ok()
    }
    /// `( v, v, … )`.
    fn arg_list(&mut self) -> Result<Vec<Value>, String> {
        self.expect(b'(')?;
        let mut out = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b')') {
            self.i += 1;
            return Ok(out);
        }
        loop {
            out.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b')') => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => return Err(self.err("expected ',' or ')'")),
            }
        }
    }
    fn value(&mut self) -> Result<Value, String> {
        self.skip_ws();
        let c = self
            .peek()
            .ok_or_else(|| self.err("unexpected end of file"))?;
        match c {
            b'$' => {
                self.i += 1;
                Ok(Value::Null)
            }
            b'*' => {
                self.i += 1;
                Ok(Value::Derived)
            }
            b'#' => {
                self.i += 1;
                self.uint()
                    .map(Value::Ref)
                    .ok_or_else(|| self.err("expected reference id"))
            }
            b'\'' => self.string().map(Value::Str),
            b'"' => {
                let s = self.i + 1;
                let e = s + self.b[s..]
                    .iter()
                    .position(|&c| c == b'"')
                    .ok_or_else(|| self.err("unterminated binary"))?;
                self.i = e + 1;
                Ok(Value::Binary(
                    String::from_utf8_lossy(&self.b[s..e]).into_owned(),
                ))
            }
            b'(' => self.arg_list().map(Value::List),
            b'.' if self
                .b
                .get(self.i + 1)
                .is_some_and(|c| c.is_ascii_alphabetic() || *c == b'_') =>
            {
                let s = self.i + 1;
                let e = s + self.b[s..]
                    .iter()
                    .position(|&c| c == b'.')
                    .ok_or_else(|| self.err("unterminated enumeration"))?;
                self.i = e + 1;
                Ok(Value::Enum(
                    String::from_utf8_lossy(&self.b[s..e]).to_ascii_uppercase(),
                ))
            }
            b'-' | b'+' | b'.' | b'0'..=b'9' => self.number(),
            c if c.is_ascii_alphabetic() || c == b'!' => {
                let name = self.keyword()?;
                let mut args = self.arg_list()?;
                let inner = if args.len() == 1 {
                    args.pop().unwrap_or(Value::Null)
                } else {
                    Value::List(args)
                };
                Ok(Value::Typed(name, Box::new(inner)))
            }
            _ => Err(self.err(&format!("unexpected character '{}'", c as char))),
        }
    }
    fn number(&mut self) -> Result<Value, String> {
        let s = self.i;
        let mut real = false;
        if matches!(self.peek(), Some(b'-' | b'+')) {
            self.i += 1;
        }
        while let Some(c) = self.peek() {
            match c {
                b'0'..=b'9' => self.i += 1,
                b'.' => {
                    real = true;
                    self.i += 1;
                }
                b'E' | b'e' => {
                    real = true;
                    self.i += 1;
                    if matches!(self.peek(), Some(b'-' | b'+')) {
                        self.i += 1;
                    }
                }
                _ => break,
            }
        }
        let t = std::str::from_utf8(&self.b[s..self.i]).map_err(|_| self.err("bad number"))?;
        if real {
            // Rust does not accept "1.E-05"; insert the missing zero.
            let fixed;
            let t = if t.contains(".E") || t.contains(".e") || t.ends_with('.') {
                fixed = t.replacen(".E", ".0E", 1).replacen(".e", ".0e", 1);
                fixed.trim_end_matches('.')
            } else {
                t
            };
            t.parse::<f64>()
                .map(Value::Real)
                .map_err(|_| self.err(&format!("bad real '{t}'")))
        } else {
            match t.parse::<i64>() {
                Ok(v) => Ok(Value::Int(v)),
                Err(_) => t
                    .parse::<f64>()
                    .map(Value::Real)
                    .map_err(|_| self.err(&format!("bad integer '{t}'"))),
            }
        }
    }
    /// A quoted string, escapes decoded.
    fn string(&mut self) -> Result<String, String> {
        self.i += 1;
        let s = self.i;
        loop {
            match self.b.get(self.i) {
                None => return Err(self.err("unterminated string")),
                Some(b'\'') if self.b.get(self.i + 1) == Some(&b'\'') => self.i += 2,
                Some(b'\'') => break,
                _ => self.i += 1,
            }
        }
        let raw = &self.b[s..self.i];
        self.i += 1;
        Ok(decode_string(raw))
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn hex_val(c: u8) -> Option<u32> {
    (c as char).to_digit(16)
}

/// Decodes ISO 10303-21 §6.4.3 string escapes and `''`.
pub fn decode_string(raw: &[u8]) -> String {
    if !raw.iter().any(|&c| c == b'\\' || c == b'\'') {
        return String::from_utf8_lossy(raw).into_owned();
    }
    let mut out = String::with_capacity(raw.len());
    let mut plain: Vec<u8> = Vec::new();
    let flush = |plain: &mut Vec<u8>, out: &mut String| {
        if !plain.is_empty() {
            out.push_str(&String::from_utf8_lossy(plain));
            plain.clear();
        }
    };
    let mut i = 0;
    while i < raw.len() {
        let c = raw[i];
        if c == b'\'' && raw.get(i + 1) == Some(&b'\'') {
            plain.push(b'\'');
            i += 2;
            continue;
        }
        if c != b'\\' {
            plain.push(c);
            i += 1;
            continue;
        }
        let rest = &raw[i..];
        if rest.starts_with(b"\\\\") {
            plain.push(b'\\');
            i += 2;
        } else if rest.starts_with(b"\\X2\\") || rest.starts_with(b"\\X4\\") {
            flush(&mut plain, &mut out);
            let wide = rest[2] == b'4';
            let mut j = i + 4;
            let mut units: Vec<u32> = Vec::new();
            let n = if wide { 8 } else { 4 };
            while j + n <= raw.len() && raw[j] != b'\\' {
                let v = raw[j..j + n]
                    .iter()
                    .try_fold(0u32, |a, &h| hex_val(h).map(|d| a * 16 + d));
                match v {
                    Some(v) => units.push(v),
                    None => break,
                }
                j += n;
            }
            if wide {
                out.extend(
                    units
                        .iter()
                        .map(|&u| char::from_u32(u).unwrap_or('\u{FFFD}')),
                );
            } else {
                let u16s: Vec<u16> = units.iter().map(|&u| u as u16).collect();
                out.push_str(&String::from_utf16_lossy(&u16s));
            }
            i = if raw[j..].starts_with(b"\\X0\\") {
                j + 4
            } else {
                j
            };
        } else if rest.starts_with(b"\\X\\") && rest.len() >= 5 {
            match (hex_val(rest[3]), hex_val(rest[4])) {
                (Some(h), Some(l)) => {
                    flush(&mut plain, &mut out);
                    out.push(char::from_u32(h * 16 + l).unwrap_or('\u{FFFD}'));
                    i += 5;
                }
                _ => {
                    plain.push(c);
                    i += 1;
                }
            }
        } else if rest.starts_with(b"\\S\\") && rest.len() >= 4 {
            flush(&mut plain, &mut out);
            out.push(char::from_u32(rest[3] as u32 + 128).unwrap_or('\u{FFFD}'));
            i += 4;
        } else if rest.len() >= 4 && rest[1] == b'P' && rest[3] == b'\\' {
            i += 4; // code-page switch \PA\ … : ignored (ISO 8859-1 assumed)
        } else {
            plain.push(c);
            i += 1;
        }
    }
    flush(&mut plain, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNIPPET: &str = r#"ISO-10303-21;
HEADER;
FILE_DESCRIPTION(('ViewDefinition [ReferenceView]'),'2;1');
FILE_NAME('brug.ifc','2026-11-05T10:00:00',('Open Infra'),(''),'rekenkern','rekenkern','');
FILE_SCHEMA(('IFC4X3_ADD2'));
ENDSEC;
/* a comment before data */
DATA;
#1=IFCPROJECT('0YvctVUKr0kugbFTf53O9L',$,'Brug \X2\00EB\X0\n',$,$,$,$,(#20),#7);
#2=IFCCARTESIANPOINT((155000.,463000.5,-1.E-05));
#3=IFCPROPERTYSINGLEVALUE('Fire''rating',$,IFCLABEL('EI 60'),$);
#4=IFCWALL('2O2Fr$t4X7Zf8NOew3FLOH',#5,'Wand /* geen commentaar */',
   'Twee
   regels',$,
   /* commentaar */ #2, *,'T\X\E9',.SOLIDWALL.);
#5=IFCPROPERTYLISTVALUE('L',$,(IFCINTEGER(3),IFCREAL(2.5E+2),IFCBOOLEAN(.T.)),$);
#6=IFCDUMMY(((1,2),(3,4)),.U.,"0FF",'\S\i',-7);
#7=IFCUNITASSIGNMENT((#2,#3));
ENDSEC;
END-ISO-10303-21;
"#;

    #[test]
    fn parses_all_argument_kinds() {
        let m = StepModel::parse_str(SNIPPET).unwrap();
        assert_eq!(m.schema().as_deref(), Some("IFC4X3_ADD2"));
        assert_eq!(m.header.file_name.as_deref(), Some("brug.ifc"));
        assert_eq!(m.len(), 7);
        let p = m.get(1).unwrap();
        assert_eq!(p.name, "IFCPROJECT");
        assert_eq!(p.str_at(2), Some("Brug ën"));
        assert!(p.arg(1).unwrap().is_null());
        assert_eq!(p.refs_at(7), vec![20]);
        assert_eq!(p.ref_at(8), Some(7));
        let pt = m.get(2).unwrap().list_at(0).unwrap();
        assert_eq!(pt[0].as_f64(), Some(155000.0));
        assert_eq!(pt[1].as_f64(), Some(463000.5));
        assert_eq!(pt[2].as_f64(), Some(-1e-5));
        let prop = m.get(3).unwrap();
        assert_eq!(prop.str_at(0), Some("Fire'rating"));
        assert_eq!(prop.arg(2).unwrap().type_name(), Some("IFCLABEL"));
        assert_eq!(prop.str_at(2), Some("EI 60"));
        let l = m.get(5).unwrap().list_at(2).unwrap();
        assert_eq!(l[0].as_i64(), Some(3));
        assert_eq!(l[1].as_f64(), Some(250.0));
        assert_eq!(l[2].as_bool(), Some(true));
        let d = m.get(6).unwrap();
        assert_eq!(
            d.list_at(0).unwrap()[1].as_list().unwrap()[0].as_i64(),
            Some(3)
        );
        assert_eq!(d.enum_at(1), Some("U"));
        assert_eq!(d.arg(1).unwrap().as_bool(), None);
        assert_eq!(d.arg(2), Some(&Value::Binary("0FF".into())));
        assert_eq!(d.str_at(3), Some("é"));
        assert_eq!(d.arg(4).unwrap().as_i64(), Some(-7));
    }

    #[test]
    fn multi_line_entity_and_comments() {
        let m = StepModel::parse_str(SNIPPET).unwrap();
        let w = m.get(4).unwrap();
        assert_eq!(w.args.len(), 9);
        assert_eq!(w.str_at(2), Some("Wand /* geen commentaar */"));
        assert!(w.str_at(3).unwrap().contains('\n'));
        assert_eq!(w.ref_at(5), Some(2));
        assert_eq!(w.arg(6), Some(&Value::Derived));
        assert_eq!(w.str_at(7), Some("Té"));
        assert_eq!(w.enum_at(8), Some("SOLIDWALL"));
    }

    #[test]
    fn by_type_and_inverse() {
        let m = StepModel::parse_str(SNIPPET).unwrap();
        assert_eq!(m.by_type("IfcWall").len(), 1);
        let mut r = m.referencing(2).to_vec();
        r.sort();
        assert_eq!(r, vec![4, 7]);
        assert_eq!(m.referencing_of_type(3, "IFCUNITASSIGNMENT").len(), 1);
        assert!(m.referencing(99).is_empty());
    }

    #[test]
    fn string_escapes() {
        assert_eq!(decode_string(br"a\\b"), "a\\b");
        assert_eq!(decode_string(br"\X2\03A9002B\X0\x"), "Ω+x");
        assert_eq!(decode_string(br"\X4\0001F600\X0\"), "😀");
        assert_eq!(decode_string(br"\X2\D83DDE00\X0\"), "😀");
        assert_eq!(decode_string(br"caf\X\E9"), "café");
        assert_eq!(decode_string(br"\PA\\S\D"), "Ä");
        assert_eq!(decode_string("it''s ü".as_bytes()), "it's ü");
    }

    /// `cargo test -p rekenkern --release -- --ignored step_speed --nocapture`
    #[test]
    #[ignore]
    fn step_speed() {
        let mut s = String::from("ISO-10303-21;HEADER;FILE_SCHEMA(('IFC4X3_ADD2'));ENDSEC;DATA;\n");
        let mut id = 1;
        while s.len() < 50_000_000 {
            s.push_str(&format!(
                "#{id}=IFCCARTESIANPOINT((155{id}.125,463000.5,-1.E-05));\n#{}=IFCWALL('2O2Fr$t4X7Zf8NOew3FLOH',$,'Wand \\X2\\00EB\\X0\\',$,$,#{id},$,'T',.SOLIDWALL.);\n",
                id + 1
            ));
            id += 2;
        }
        s.push_str("ENDSEC;END-ISO-10303-21;\n");
        let t = std::time::Instant::now();
        let m = StepModel::parse_str(&s).unwrap();
        println!(
            "{} MB, {} instances in {:?}",
            s.len() / 1_000_000,
            m.len(),
            t.elapsed()
        );
        assert_eq!(m.len(), id as usize - 1);
    }

    #[test]
    fn complex_instance_and_errors() {
        let t = "ISO-10303-21;HEADER;FILE_SCHEMA(('IFC4'));ENDSEC;DATA;#1=(A(1)B('x'));ENDSEC;END-ISO-10303-21;";
        let m = StepModel::parse_str(t).unwrap();
        assert_eq!(m.get(1).unwrap().name, "A+B");
        assert_eq!(m.get(1).unwrap().args.len(), 2);
        let e = StepModel::parse_str("ISO-10303-21;\nHEADER;ENDSEC;DATA;\n#1=X(1,;\nENDSEC;")
            .unwrap_err();
        assert!(e.contains("line 3"), "{e}");
        assert!(StepModel::parse_str("hello").is_err());
    }
}
