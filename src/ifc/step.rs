//! ISO 10303-21 (STEP physical file) output: instance numbers, the value
//! syntax of IFC files and IFC GlobalIds.

use std::fmt::Write as _;
use std::io::{self, Write};

/// An instance number, written as `#n`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id(pub u64);

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// The DATA section, written as instances are added.
pub struct Step<W: Write> {
    out: W,
    next: u64,
}

impl<W: Write> Step<W> {
    /// Header and the start of the DATA section.
    pub fn new(mut out: W, view: &str, name: &str, time: &str, author: &str, organisation: &str, system: &str, schema: &str) -> io::Result<Self> {
        writeln!(out, "ISO-10303-21;")?;
        writeln!(out, "HEADER;")?;
        writeln!(out, "FILE_DESCRIPTION(({}),'2;1');", text(&format!("ViewDefinition [{view}]")))?;
        writeln!(out, "FILE_NAME({},{},({}),({}),{},{},'');", text(name), text(time), text(author), text(organisation), text(system), text(system))?;
        writeln!(out, "FILE_SCHEMA(({}));", text(schema))?;
        writeln!(out, "ENDSEC;")?;
        writeln!(out, "DATA;")?;
        Ok(Step { out, next: 1 })
    }

    /// `#n=ENTITY(args);` with `args` already in STEP syntax.
    pub fn add(&mut self, entity: &str, args: &str) -> io::Result<Id> {
        let id = Id(self.next);
        self.next += 1;
        writeln!(self.out, "{id}={entity}({args});")?;
        Ok(id)
    }

    /// The number of instances written so far.
    pub fn count(&self) -> u64 {
        self.next - 1
    }

    pub fn finish(mut self) -> io::Result<W> {
        writeln!(self.out, "ENDSEC;")?;
        writeln!(self.out, "END-ISO-10303-21;")?;
        self.out.flush()?;
        Ok(self.out)
    }
}

/// A REAL: the shortest form that reads back, always with a decimal point.
pub fn real(x: f64) -> String {
    if !x.is_finite() || x == 0.0 {
        return "0.".into();
    }
    let mut s = format!("{x}");
    if !s.contains('.') {
        s.push('.');
    }
    s
}

/// A coordinate in metres, to the micrometre.
pub fn coord(x: f64) -> String {
    if !x.is_finite() {
        return "0.".into();
    }
    let s = format!("{x:.6}");
    let s = s.trim_end_matches('0');
    match s {
        "-." | "." | "-0." => "0.".into(),
        _ => s.to_string(),
    }
}

/// A STRING: quotes and backslashes doubled, everything outside printable
/// ASCII as UTF-16 in `\X2\…\X0\`.
pub fn text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    let mut wide: Vec<u16> = Vec::new();
    for c in s.chars() {
        if (' '..='~').contains(&c) {
            flush_wide(&mut out, &mut wide);
            match c {
                '\'' => out.push_str("''"),
                '\\' => out.push_str("\\\\"),
                _ => out.push(c),
            }
        } else {
            let mut buf = [0u16; 2];
            wide.extend_from_slice(c.encode_utf16(&mut buf));
        }
    }
    flush_wide(&mut out, &mut wide);
    out.push('\'');
    out
}

fn flush_wide(out: &mut String, wide: &mut Vec<u16>) {
    if wide.is_empty() {
        return;
    }
    out.push_str("\\X2\\");
    for u in wide.drain(..) {
        let _ = write!(out, "{u:04X}");
    }
    out.push_str("\\X0\\");
}

/// An optional STRING: `$` when absent or empty.
pub fn opt_text(s: Option<&str>) -> String {
    match s {
        Some(s) if !s.is_empty() => text(s),
        _ => "$".into(),
    }
}

/// `(#1,#2,…)`.
pub fn refs(ids: &[Id]) -> String {
    let mut s = String::with_capacity(ids.len() * 8 + 2);
    s.push('(');
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        let _ = write!(s, "{id}");
    }
    s.push(')');
    s
}

/// `(a,b,…)` of reals.
pub fn reals(v: &[f64]) -> String {
    let parts: Vec<String> = v.iter().map(|x| real(*x)).collect();
    format!("({})", parts.join(","))
}

/// `((x,y,z),…)` of coordinates.
pub fn coord_list<const N: usize>(pts: &[[f64; N]]) -> String {
    let mut s = String::with_capacity(pts.len() * N * 12 + 2);
    s.push('(');
    for (i, p) in pts.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('(');
        for (j, c) in p.iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            s.push_str(&coord(*c));
        }
        s.push(')');
    }
    s.push(')');
    s
}

/// `((1,2,3),…)` of indices (1-based in IFC).
pub fn index_list<const N: usize>(v: &[[usize; N]]) -> String {
    let mut s = String::with_capacity(v.len() * N * 6 + 2);
    s.push('(');
    for (i, t) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('(');
        for (j, k) in t.iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            let _ = write!(s, "{}", k + 1);
        }
        s.push(')');
    }
    s.push(')');
    s
}

// ── GlobalId ─────────────────────────────────────────────────────────────────

const B64: &[u8; 64] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_$";

/// A GlobalId that stays the same for the same object in the same drawing,
/// so a second export of an unchanged drawing gives the same ids: a
/// version-4 style UUID from a hash of `seed` and `key`, in the 22
/// characters of the IFC base-64 alphabet.
pub fn guid(seed: &str, key: &str) -> String {
    let h1 = mix(fnv(0xcbf2_9ce4_8422_2325, seed, key));
    let h2 = mix(fnv(0x6c62_272e_07bb_0142, key, seed) ^ h1.rotate_left(29));
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&h1.to_be_bytes());
    b[8..].copy_from_slice(&h2.to_be_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    compress(&b)
}

fn fnv(basis: u64, a: &str, b: &str) -> u64 {
    let mut h = basis;
    for byte in a.bytes().chain(std::iter::once(0)).chain(b.bytes()) {
        h ^= byte as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// splitmix64's finaliser: spreads FNV's weak low bits over the word.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// 16 bytes → 22 characters: the first byte in 2, then 5 × 3 bytes in 4 each.
pub fn compress(b: &[u8; 16]) -> String {
    let mut s = String::with_capacity(22);
    let mut put = |v: u32, n: u32| {
        for i in (0..n).rev() {
            s.push(B64[((v >> (6 * i)) & 63) as usize] as char);
        }
    };
    put(b[0] as u32, 2);
    for i in (1..16).step_by(3) {
        put(((b[i] as u32) << 16) | ((b[i + 1] as u32) << 8) | b[i + 2] as u32, 4);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reals_always_have_a_point() {
        assert_eq!(real(1.0), "1.");
        assert_eq!(real(-0.0), "0.");
        assert_eq!(real(0.25), "0.25");
        assert_eq!(real(1e-7), "0.0000001");
        assert_eq!(coord(155000.1234567), "155000.123457");
        assert_eq!(coord(12.0), "12.");
        assert_eq!(coord(-0.0000001), "0.");
        assert_eq!(coord(-3.5), "-3.5");
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(text("it's"), "'it''s'");
        assert_eq!(text("a\\b"), "'a\\\\b'");
        assert_eq!(text("Grote Kerk — Dordrecht"), "'Grote Kerk \\X2\\2014\\X0\\ Dordrecht'");
        assert_eq!(text("é"), "'\\X2\\00E9\\X0\\'");
        assert_eq!(opt_text(Some("")), "$");
    }

    #[test]
    fn guids_are_22_characters_and_stable() {
        let a = guid("drawing.dwg", "entity:1A2B");
        assert_eq!(a.len(), 22);
        assert_eq!(a, guid("drawing.dwg", "entity:1A2B"));
        assert_ne!(a, guid("drawing.dwg", "entity:1A2C"));
        assert!(a.bytes().all(|c| B64.contains(&c)));
        // The first character holds only the top two bits of the first byte.
        assert!("0123".contains(&a[..1]));
        // The ifcopenshell reference: 00000000-0000-0000-0000-000000000000 → 22 zeros.
        assert_eq!(compress(&[0; 16]), "0".repeat(22));
        assert_eq!(compress(&[255; 16]), format!("3{}", "$".repeat(21)));
    }

    #[test]
    fn lists() {
        assert_eq!(coord_list(&[[0.0, 1.5, 2.0]]), "((0.,1.5,2.))");
        assert_eq!(index_list(&[[0usize, 1, 2]]), "((1,2,3))");
        assert_eq!(refs(&[Id(3), Id(7)]), "(#3,#7)");
    }
}
