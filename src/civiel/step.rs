//! A small writer for STEP physical files (ISO 10303-21), enough for IFC:
//! numbered entity instances with their attributes, the header and the
//! encoding of strings. No schema knowledge: the IFC writer (`ifc.rs`)
//! knows the entities and the order of their attributes.

use std::fmt::Write as _;

/// An entity instance: `#n`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub u64);

/// One attribute value.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    /// `$`: not given (an OPTIONAL attribute).
    Null,
    /// `*`: an attribute derived in a subtype.
    Derived,
    Int(i64),
    Real(f64),
    /// `.T.` or `.F.`.
    Bool(bool),
    /// A string, encoded on writing.
    Str(String),
    /// `.NAME.`: an enumeration value.
    Enum(&'static str),
    Ref(Id),
    List(Vec<V>),
    /// `IFCLENGTHMEASURE(1.)`: a value of a defined type in a SELECT.
    Typed(&'static str, Box<V>),
}

impl From<Id> for V {
    fn from(id: Id) -> V {
        V::Ref(id)
    }
}

impl From<f64> for V {
    fn from(x: f64) -> V {
        V::Real(x)
    }
}

impl From<&str> for V {
    fn from(s: &str) -> V {
        V::Str(s.to_string())
    }
}

impl From<String> for V {
    fn from(s: String) -> V {
        V::Str(s)
    }
}

impl From<Option<Id>> for V {
    fn from(id: Option<Id>) -> V {
        id.map_or(V::Null, V::Ref)
    }
}

/// A list of references.
pub fn refs(ids: &[Id]) -> V {
    V::List(ids.iter().map(|i| V::Ref(*i)).collect())
}

/// A list of reals.
pub fn reals(xs: &[f64]) -> V {
    V::List(xs.iter().map(|x| V::Real(*x)).collect())
}

/// A string, or `$` when empty.
pub fn opt_str(s: &str) -> V {
    if s.is_empty() { V::Null } else { V::Str(s.to_string()) }
}

/// A real as STEP writes it: always with a decimal point ("1.", "0.25",
/// "-3.5E-07"); no NaN or infinity (written as 0).
pub fn real(x: f64) -> String {
    if !x.is_finite() {
        return "0.".into();
    }
    if x == 0.0 {
        return "0.".into();
    }
    let a = x.abs();
    if (1e-4..1e15).contains(&a) {
        // Rust's shortest round-trip form has no exponent in this range.
        let s = format!("{x}");
        if s.contains('.') { s } else { format!("{s}.") }
    } else {
        // "1.5e-7" → "1.5E-07".
        let s = format!("{x:e}");
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = if m.contains('.') { m.to_string() } else { format!("{m}.") };
        let e: i32 = e.parse().unwrap_or(0);
        format!("{m}E{}{:02}", if e < 0 { "-" } else { "" }, e.abs())
    }
}

/// A string literal: `'…'` with `'` and `\` doubled, and everything outside
/// printable ASCII as `\X2\hhhh…\X0\` (UTF-16, ISO 10303-21 2nd edition).
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    let mut wide: Vec<u16> = Vec::new();
    let flush = |wide: &mut Vec<u16>, out: &mut String| {
        if !wide.is_empty() {
            out.push_str("\\X2\\");
            for u in wide.iter() {
                let _ = write!(out, "{u:04X}");
            }
            out.push_str("\\X0\\");
            wide.clear();
        }
    };
    for c in s.chars() {
        if (' '..='~').contains(&c) {
            flush(&mut wide, &mut out);
            match c {
                '\'' => out.push_str("''"),
                '\\' => out.push_str("\\\\"),
                c => out.push(c),
            }
        } else {
            let mut buf = [0u16; 2];
            wide.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush(&mut wide, &mut out);
    out.push('\'');
    out
}

fn value(v: &V, out: &mut String) {
    match v {
        V::Null => out.push('$'),
        V::Derived => out.push('*'),
        V::Int(i) => {
            let _ = write!(out, "{i}");
        }
        V::Real(x) => out.push_str(&real(*x)),
        V::Bool(b) => out.push_str(if *b { ".T." } else { ".F." }),
        V::Str(s) => out.push_str(&string(s)),
        V::Enum(e) => {
            out.push('.');
            out.push_str(e);
            out.push('.');
        }
        V::Ref(id) => {
            let _ = write!(out, "#{}", id.0);
        }
        V::List(items) => {
            out.push('(');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                value(x, out);
            }
            out.push(')');
        }
        V::Typed(t, x) => {
            out.push_str(t);
            out.push('(');
            value(x, out);
            out.push(')');
        }
    }
}

/// The DATA section, built instance by instance.
#[derive(Default)]
pub struct Step {
    data: String,
    next: u64,
}

impl Step {
    pub fn new() -> Step {
        Step { data: String::new(), next: 1 }
    }

    /// Add `#n=ENTITY(attributes);` and return `#n`. The entity name is
    /// written in capitals.
    pub fn add(&mut self, entity: &str, attrs: Vec<V>) -> Id {
        let id = Id(self.next);
        self.next += 1;
        let _ = write!(self.data, "#{}={}", id.0, entity.to_ascii_uppercase());
        value(&V::List(attrs), &mut self.data);
        self.data.push_str(";\n");
        id
    }

    /// Instances written so far.
    pub fn count(&self) -> u64 {
        self.next - 1
    }

    /// The whole file: header, data, end.
    pub fn finish(self, header: &Header) -> String {
        let mut out = String::with_capacity(self.data.len() + 600);
        out.push_str("ISO-10303-21;\nHEADER;\n");
        let desc = V::List(header.description.iter().map(|d| V::Str(d.clone())).collect());
        let mut line = String::new();
        value(&desc, &mut line);
        let _ = writeln!(out, "FILE_DESCRIPTION({line},'2;1');");
        let _ = writeln!(
            out,
            "FILE_NAME({},{},({}),({}),{},{},'');",
            string(&header.name),
            string(&header.time_stamp),
            string(&header.author),
            string(&header.organization),
            string(&header.preprocessor),
            string(&header.originating_system),
        );
        let _ = writeln!(out, "FILE_SCHEMA(({}));", string(&header.schema));
        out.push_str("ENDSEC;\nDATA;\n");
        out.push_str(&self.data);
        out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
        out
    }
}

pub struct Header {
    pub description: Vec<String>,
    pub name: String,
    /// ISO 8601, e.g. "2026-10-29T12:00:00".
    pub time_stamp: String,
    pub author: String,
    pub organization: String,
    pub preprocessor: String,
    pub originating_system: String,
    pub schema: String,
}

// ── IFC GUIDs ──────────────────────────────────────────────────────────────

const GUID_CHARS: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";

/// The 22-character IFC form of a 128-bit GUID (IfcGloballyUniqueId): the
/// first byte in two characters, then five groups of three bytes in four.
pub fn compress_guid(bytes: [u8; 16]) -> String {
    let b64 = |v: u32, n: usize, out: &mut String| {
        for i in (0..n).rev() {
            out.push(GUID_CHARS[((v >> (6 * i)) & 63) as usize] as char);
        }
    };
    let mut out = String::with_capacity(22);
    b64(bytes[0] as u32, 2, &mut out);
    for i in (1..16).step_by(3) {
        b64(((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | bytes[i + 2] as u32, 4, &mut out);
    }
    out
}

/// A stable GUID from a namespace and a key (two FNV-1a hashes, laid out as
/// a version 5 UUID): the same object gets the same GUID in every export of
/// the same drawing, so a receiving model can follow changes.
pub fn guid(namespace: &str, key: &str) -> String {
    let fnv = |seed: u64| {
        let mut h: u64 = 0xcbf29ce484222325 ^ seed;
        for b in namespace.bytes().chain([0u8]).chain(key.bytes()) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    };
    let (a, b) = (fnv(0x9e3779b97f4a7c15), fnv(0x632be59bd9b4e019));
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&a.to_be_bytes());
    bytes[8..].copy_from_slice(&b.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    compress_guid(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reals_always_have_a_point() {
        assert_eq!(real(0.0), "0.");
        assert_eq!(real(1.0), "1.");
        assert_eq!(real(-2.5), "-2.5");
        assert_eq!(real(104885.125), "104885.125");
        assert_eq!(real(1e-5), "1.E-05");
        assert_eq!(real(-3.5e-7), "-3.5E-07");
        assert_eq!(real(2.5e20), "2.5E20");
        assert_eq!(real(f64::NAN), "0.");
        // Parse back: nothing is lost.
        for x in [0.1, 1.0 / 3.0, 425_320.000_1, 1e-9, 6.02e23] {
            let s = real(x).replace("E", "e");
            let s = s.replace(".e", ".0e");
            assert_eq!(s.parse::<f64>().unwrap(), x, "{s}");
        }
    }

    #[test]
    fn strings_are_encoded() {
        assert_eq!(string("As 1"), "'As 1'");
        assert_eq!(string("Jan's \\pad"), "'Jan''s \\\\pad'");
        // ë and € in one \X2\ run, then plain text again.
        assert_eq!(string("Zeeë€ k"), "'Zee\\X2\\00EB20AC\\X0\\ k'");
        // Outside the basic plane: a surrogate pair.
        assert_eq!(string("🙂"), "'\\X2\\D83DDE42\\X0\\'");
    }

    #[test]
    fn instances_and_file() {
        let mut s = Step::new();
        let p = s.add("IfcCartesianPoint", vec![reals(&[0.0, 1.5, -2.0])]);
        let d = s.add("IfcDirection", vec![reals(&[1.0, 0.0])]);
        let a = s.add("IfcAxis2Placement3D", vec![p.into(), V::Null, d.into()]);
        let u = s.add("IfcSIUnit", vec![V::Derived, V::Enum("LENGTHUNIT"), V::Null, V::Enum("METRE")]);
        let m = s.add("IfcPointByDistanceExpression", vec![V::Typed("IFCLENGTHMEASURE", Box::new(V::Real(12.0))), V::Null, V::Null, V::Null, a.into()]);
        assert_eq!((a, u, m, s.count()), (Id(3), Id(4), Id(5), 5));
        let text = s.finish(&Header {
            description: vec!["ViewDefinition [ReferenceView]".into()],
            name: "test.ifc".into(),
            time_stamp: "2026-10-29T12:00:00".into(),
            author: String::new(),
            organization: String::new(),
            preprocessor: "Open Infra".into(),
            originating_system: "Open Infra".into(),
            schema: "IFC4X3_ADD2".into(),
        });
        assert!(text.starts_with("ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('ViewDefinition [ReferenceView]'),'2;1');\n"));
        assert!(text.contains("FILE_SCHEMA(('IFC4X3_ADD2'));"));
        assert!(text.contains("#1=IFCCARTESIANPOINT((0.,1.5,-2.));\n"));
        assert!(text.contains("#3=IFCAXIS2PLACEMENT3D(#1,$,#2);\n"));
        assert!(text.contains("#4=IFCSIUNIT(*,.LENGTHUNIT.,$,.METRE.);\n"));
        assert!(text.contains("#5=IFCPOINTBYDISTANCEEXPRESSION(IFCLENGTHMEASURE(12.),$,$,$,#3);\n"));
        assert!(text.ends_with("ENDSEC;\nEND-ISO-10303-21;\n"));
    }

    #[test]
    fn guids() {
        // The all-ones GUID compresses to the largest value: "3$$$…".
        assert_eq!(compress_guid([0xff; 16]), "3$$$$$$$$$$$$$$$$$$$$$");
        assert_eq!(compress_guid([0; 16]), "0000000000000000000000");
        // A known pair (ifcopenshell.guid.compress of 0123…cdef).
        let b = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
        assert_eq!(compress_guid(b), "018qLdYQlDxm4ZHMU9gytl");
        let g = guid("tekening", "as/As 1");
        assert_eq!(g.len(), 22);
        assert!(matches!(g.as_bytes()[0], b'0'..=b'3'));
        assert_eq!(g, guid("tekening", "as/As 1"), "stable");
        assert_ne!(g, guid("tekening", "as/As 2"));
        assert_ne!(g, guid("andere", "as/As 1"));
    }
}
