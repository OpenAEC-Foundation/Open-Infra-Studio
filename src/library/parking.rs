//! Parking bays along a line: rows of perpendicular, angled or parallel bays
//! on both sides of an aisle (the line is the aisle's centre), numbered, with
//! the dimensions of NEN 2443:2013 per parking angle and facility class, and
//! optionally one bay checked with a swept path.
//!
//! NEN 2443:2013 ("Parkeren en stallen van personenauto's op terreinen en in
//! garages"), normtekst 5.3.8 and tables 4A (public), 4B (non-public) and 4C
//! (storage), as reproduced in the annex of the municipality of Maastricht's
//! "Beleidsregel parkeren en duurzaamheid Belvédère" (CVDR654663). The values
//! below are those of that reproduction; the standard itself is not public.
//!
//! - P1: depth of a single row of bays, measured square to the aisle.
//! - W: width of the aisle (parkeerweg).
//! - b: bay width, measured square to the bay's long sides.

use ocs_plugin_api::host::codec::entities::{TextHorizontalAlignment, TextVerticalAlignment};
use ocs_plugin_api::host::codec::{Color, EntityType, LwPolyline, Text, Vector2, Vector3};
use ocs_plugin_api::host::{CommandStep, HostApi, InteractiveCommand, PreviewWire};

use super::Vehicle;
use crate::bridge;
use crate::lang::tr;

/// A message in the interface language (Dutch or English).
macro_rules! t {
    ($nl:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if crate::lang::current() == crate::lang::Lang::Nl { format!($nl $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

type P = [f64; 2];

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
fn mul(a: P, k: f64) -> P {
    [a[0] * k, a[1] * k]
}
fn len(a: P) -> f64 {
    a[0].hypot(a[1])
}
fn unit(a: P) -> P {
    let l = len(a).max(1e-12);
    [a[0] / l, a[1] / l]
}
fn left(d: P) -> P {
    [-d[1], d[0]]
}
fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

/// The facility classes of NEN 2443 (its tables 4A, 4B, 4C), with "intensive
/// public use", which 5.3.8.1 gives a bay width of at least 2.50 m.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Public, intensive use (shops, stations): table 4A, b ≥ 2.50 m.
    Intensive,
    /// Public car parks: table 4A.
    Public,
    /// Non-public (offices, residents): table 4B.
    NonPublic,
    /// Storage (fixed users, own spaces): table 4C.
    Storage,
}

impl Class {
    pub fn parse(s: &str) -> Option<Class> {
        match s.to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
            "intensief" | "openbaarintensief" | "intensive" => Some(Class::Intensive),
            "openbaar" | "public" | "4a" => Some(Class::Public),
            "nietopenbaar" | "nonpublic" | "private" | "4b" => Some(Class::NonPublic),
            "stalling" | "storage" | "4c" => Some(Class::Storage),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Class::Intensive => tr("openbaar, intensief gebruik", "public, intensive use"),
            Class::Public => tr("openbaar", "public"),
            Class::NonPublic => tr("niet-openbaar", "non-public"),
            Class::Storage => tr("stalling", "storage"),
        }
    }

    pub fn table(self) -> &'static str {
        match self {
            Class::Intensive | Class::Public => "4A",
            Class::NonPublic => "4B",
            Class::Storage => "4C",
        }
    }

    /// The smallest bay width NEN 2443 5.3.8.1 allows.
    fn min_width(self) -> f64 {
        match self {
            Class::Intensive => 2.50,
            Class::Public => 2.40,
            Class::NonPublic | Class::Storage => 2.30,
        }
    }
}

/// One row of a table: angle, then per bay width (P1, W).
type Row = (f64, &'static [(f64, f64, f64)]);

/// NEN 2443:2013 table 4A: public car parks and garages. (b, P1, W).
const TABLE_4A: &[Row] = &[
    (30.0, &[(2.40, 4.19, 3.80), (2.45, 4.19, 3.80), (2.50, 4.19, 3.80)]),
    (45.0, &[(2.40, 4.95, 3.80), (2.45, 4.95, 3.80), (2.50, 4.95, 3.80)]),
    (60.0, &[(2.40, 5.38, 3.80), (2.45, 5.38, 3.80), (2.50, 5.38, 3.80), (2.55, 5.38, 3.80)]),
    (65.0, &[(2.40, 5.44, 3.87), (2.45, 5.44, 3.80), (2.50, 5.44, 3.80), (2.55, 5.44, 3.80)]),
    (70.0, &[(2.40, 5.46, 4.43), (2.45, 5.46, 4.08), (2.50, 5.46, 3.82), (2.55, 5.46, 3.80)]),
    (80.0, &[(2.40, 5.37, 5.53), (2.45, 5.37, 5.23), (2.50, 5.37, 4.87), (2.55, 5.37, 4.47)]),
    (90.0, &[(2.40, 5.13, 6.67), (2.45, 5.13, 6.33), (2.50, 5.13, 6.00), (2.55, 5.13, 5.67)]),
];

/// Table 4B: non-public car parks and garages.
const TABLE_4B: &[Row] = &[
    (30.0, &[(2.30, 4.19, 3.00), (2.35, 4.19, 3.00), (2.40, 4.19, 3.00), (2.45, 4.19, 3.00), (2.50, 4.19, 3.00)]),
    (45.0, &[(2.30, 4.95, 3.00), (2.35, 4.95, 3.00), (2.40, 4.95, 3.00), (2.45, 4.95, 3.00), (2.50, 4.95, 3.00)]),
    (60.0, &[(2.30, 5.38, 3.70), (2.35, 5.38, 3.40), (2.40, 5.38, 3.10), (2.45, 5.38, 3.00), (2.50, 5.38, 3.00)]),
    (65.0, &[(2.30, 5.44, 4.17), (2.35, 5.44, 3.80), (2.40, 5.44, 3.60), (2.45, 5.44, 3.20), (2.50, 5.44, 3.00)]),
    (70.0, &[(2.30, 5.46, 4.66), (2.35, 5.46, 4.36), (2.40, 5.46, 4.07), (2.45, 5.46, 3.80), (2.50, 5.46, 3.50)]),
    (80.0, &[(2.30, 5.38, 5.86), (2.35, 5.38, 5.47), (2.40, 5.37, 5.16), (2.45, 5.37, 4.85), (2.50, 5.37, 4.55)]),
    (90.0, &[(2.30, 5.13, 7.00), (2.35, 5.13, 6.66), (2.40, 5.13, 6.33), (2.45, 5.13, 6.00), (2.50, 5.13, 5.66)]),
];

/// Table 4C: storage car parks and garages.
const TABLE_4C: &[Row] = &[
    (30.0, &[(2.30, 4.19, 2.75), (2.35, 4.19, 2.75), (2.40, 4.19, 2.75), (2.45, 4.19, 2.75), (2.50, 4.19, 2.75)]),
    (45.0, &[(2.30, 4.95, 2.75), (2.35, 4.95, 2.75), (2.40, 4.95, 2.75), (2.45, 4.95, 2.75), (2.50, 4.95, 2.75)]),
    (60.0, &[(2.30, 5.38, 3.40), (2.35, 5.38, 3.10), (2.40, 5.38, 2.90), (2.45, 5.38, 2.75), (2.50, 5.38, 2.75)]),
    (65.0, &[(2.30, 5.44, 3.86), (2.35, 5.44, 3.60), (2.40, 5.44, 3.30), (2.45, 5.44, 3.00), (2.50, 5.44, 2.75)]),
    (70.0, &[(2.30, 5.46, 4.41), (2.35, 5.46, 4.11), (2.40, 5.46, 3.80), (2.45, 5.46, 3.50), (2.50, 5.46, 3.20)]),
    (80.0, &[(2.30, 5.38, 5.50), (2.35, 5.38, 5.17), (2.40, 5.37, 4.85), (2.45, 5.37, 4.54), (2.50, 5.37, 4.17)]),
    (90.0, &[(2.30, 5.13, 6.62), (2.35, 5.13, 6.29), (2.40, 5.13, 5.95), (2.45, 5.13, 5.62), (2.50, 5.13, 5.29)]),
];

/// The bays and aisle of one layout.
#[derive(Clone, Debug, PartialEq)]
pub struct Dims {
    /// Parking angle in degrees; 0 for parallel parking.
    pub angle: f64,
    /// Bay width b (parallel: the width of the bay, square to the aisle).
    pub width: f64,
    /// Depth of a row square to the aisle (P1; parallel: the bay width).
    pub depth: f64,
    /// Aisle width W.
    pub aisle: f64,
    /// Parallel parking: the bay length along the aisle.
    pub length: f64,
    pub class: Class,
    /// Where the values come from.
    pub source: String,
}

/// Bay and aisle dimensions per NEN 2443:2013. `angle` 0 is parallel
/// parking; `width` the bay width (a column of the table; default 2.50 m,
/// for parallel parking 2.00 m).
pub fn dimensions(class: Class, angle: f64, width: Option<f64>) -> Result<Dims, String> {
    if angle.abs() < 1e-9 {
        // 5.3.8.2 and 5.3.8.3: at least 1.80 m with exit space beside the
        // row, 2.00 m along walls; at least 6.25 m long in public car parks,
        // 5.75 m in non-public and storage garages; aisle at least 4 m,
        // 3.50 m in non-public and storage facilities.
        let w = width.unwrap_or(2.0);
        if w < 1.80 - 1e-9 {
            return Err(tr("langsparkeren: een vak is minstens 1,80 m breed (NEN 2443 5.3.8.2)", "parallel parking: a bay is at least 1.80 m wide (NEN 2443 5.3.8.2)").into());
        }
        let (length, aisle) = match class {
            Class::Intensive | Class::Public => (6.25, 4.00),
            Class::NonPublic | Class::Storage => (5.75, 3.50),
        };
        return Ok(Dims { angle: 0.0, width: w, depth: w, aisle, length, class, source: "NEN 2443:2013 5.3.8.2, 5.3.8.3".into() });
    }
    let table = match class {
        Class::Intensive | Class::Public => TABLE_4A,
        Class::NonPublic => TABLE_4B,
        Class::Storage => TABLE_4C,
    };
    let Some((_, cols)) = table.iter().find(|(a, _)| (a - angle).abs() < 1e-6) else {
        let angles: Vec<String> = table.iter().map(|(a, _)| format!("{a:.0}")).collect();
        return Err(t!("parkeerhoek {angle}° staat niet in NEN 2443 tabel {}: kies {} of LANGS", "parking angle {angle}° is not in NEN 2443 table {}: choose {} or LANGS", class.table(), angles.join(", ")));
    };
    let w = width.unwrap_or(2.50);
    if w < class.min_width() - 1e-9 {
        return Err(t!(
            "vakbreedte {w:.2} m is smaller dan de {:.2} m die NEN 2443 5.3.8.1 vraagt voor {}",
            "bay width {w:.2} m is below the {:.2} m that NEN 2443 5.3.8.1 asks for {}",
            class.min_width(),
            class.label()
        ));
    }
    let Some((b, p1, aisle)) = cols.iter().find(|(b, _, _)| (b - w).abs() < 1e-6) else {
        let widths: Vec<String> = cols.iter().map(|(b, _, _)| format!("{b:.2}")).collect();
        return Err(t!("vakbreedte {w:.2} m staat niet in tabel {} bij {angle}°: kies {}", "bay width {w:.2} m is not in table {} at {angle}°: choose {}", class.table(), widths.join(", ")));
    };
    Ok(Dims { angle, width: *b, depth: *p1, aisle: *aisle, length: 0.0, class, source: format!("NEN 2443:2013 {} {}", tr("tabel", "table"), class.table()) })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sides {
    Both,
    Left,
    Right,
}

#[derive(Clone, Debug)]
pub struct Bay {
    pub number: usize,
    /// Counter-clockwise corners; the first two on the aisle edge.
    pub corners: [P; 4],
    /// +1 left of the line, −1 right of it.
    pub side: i8,
    /// Along the line, and into the bay.
    pub along: P,
    pub into: P,
    /// The middle of the bay's front (on the aisle edge).
    pub front: P,
}

#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub bays: Vec<Bay>,
    /// Aisle edges, one pair per segment of the line.
    pub edges: Vec<[P; 2]>,
}

fn ccw(mut q: [P; 4]) -> [P; 4] {
    let a: f64 = (0..4).map(|i| cross(q[i], q[(i + 1) % 4])).sum();
    if a < 0.0 {
        q.reverse();
        // Keep the two aisle corners first.
        q = [q[2], q[3], q[0], q[1]];
    }
    q
}

/// Signed distance of `p` inside a counter-clockwise convex polygon
/// (positive inside, negative outside: the nearest edge decides).
fn inside_depth(p: P, q: &[P; 4]) -> f64 {
    (0..4)
        .map(|i| {
            let (a, b) = (q[i], q[(i + 1) % 4]);
            cross(unit(sub(b, a)), sub(p, a))
        })
        .fold(f64::MAX, f64::min)
}

/// How deep two convex quadrilaterals overlap (0 when apart): the deepest
/// corner of one inside the other.
pub fn overlap(a: &[P; 4], b: &[P; 4]) -> f64 {
    let d1 = a.iter().map(|p| inside_depth(*p, b)).fold(f64::MIN, f64::max);
    let d2 = b.iter().map(|p| inside_depth(*p, a)).fold(f64::MIN, f64::max);
    d1.max(d2).max(0.0)
}

/// Lay out the bays along `line` (metres; the aisle's centre line). Bays
/// that would overlap earlier ones (inside a bend) are left out.
pub fn layout(line: &[P], d: &Dims, sides: Sides, first: usize) -> Layout {
    let mut out = Layout::default();
    let mut n = first;
    let order: Vec<i8> = match sides {
        Sides::Both => vec![1, -1],
        Sides::Left => vec![1],
        Sides::Right => vec![-1],
    };
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let l = len(sub(b, a));
        if l < 1e-9 {
            continue;
        }
        let t = unit(sub(b, a));
        let nrm = left(t);
        for s in [1.0, -1.0] {
            out.edges.push([add(a, mul(nrm, s * d.aisle / 2.0)), add(b, mul(nrm, s * d.aisle / 2.0))]);
        }
    }
    for s in order {
        let sf = s as f64;
        for w in line.windows(2) {
            let (a, b) = (w[0], w[1]);
            let l = len(sub(b, a));
            if l < 1e-9 {
                continue;
            }
            let t = unit(sub(b, a));
            let nrm = left(t);
            let base = add(a, mul(nrm, sf * d.aisle / 2.0));
            let (into, wa, side_len, shift) = if d.angle == 0.0 {
                (mul(nrm, sf), d.length, d.width, 0.0)
            } else {
                let al = d.angle.to_radians();
                let into = add(mul(t, al.cos()), mul(nrm, sf * al.sin()));
                (into, d.width / al.sin(), d.depth / al.sin(), d.depth / al.tan())
            };
            let count = ((l - shift) / wa + 1e-9).floor().max(0.0) as usize;
            let start = (l - shift - count as f64 * wa) / 2.0;
            for k in 0..count {
                let a0 = add(base, mul(t, start + k as f64 * wa));
                let b0 = add(a0, mul(t, wa));
                let quad = ccw([a0, b0, add(b0, mul(into, side_len)), add(a0, mul(into, side_len))]);
                if out.bays.iter().any(|o| overlap(&o.corners, &quad) > 0.01) {
                    continue;
                }
                out.bays.push(Bay { number: n, corners: quad, side: s, along: t, into, front: mul(add(a0, b0), 0.5) });
                n += 1;
            }
        }
    }
    out
}

/// The rear axle path of a vehicle checking `bay`, and whether it reverses
/// along it (the rear axle of the last unit is guided, so the vehicle ends
/// straight in its bay):
/// - square bays (90°): driven past, then reversed in on a quarter circle,
///   as large as the far side of the aisle allows for the front's swing;
/// - angled bays: forward in on an arc as large as the aisle allows;
/// - parallel bays: reversed in along an S of two arcs just wider than
///   the vehicle's full lock.
///
/// The arcs are Open Infra's choice of manoeuvre, not part of NEN 2443.
pub fn entry_path(v: &Vehicle, d: &Dims, bay: &Bay) -> Option<(Vec<P>, bool)> {
    let last = v.units.last()?;
    let (_, rl) = super::swept::axles(last)?;
    // The front of the vehicle ahead of the guided axle, its rear behind it.
    let fronts = super::draw::unit_fronts(v);
    let reach = fronts[v.units.len() - 1] + rl;
    let rear_overhang = last.length - rl;
    let wv = v.width();
    let t = bay.along;
    let s = bay.side as f64;
    let nrm = left(t);
    // The tightest the vehicle can turn its rear axle (from its outer
    // turning radius, when known).
    let r_min = v.turning_radius_outer.map(|r| ((r * r - reach * reach).max(0.0)).sqrt() - wv / 2.0).filter(|r| *r > 0.5).unwrap_or(3.0);
    let arc = |c: P, from: P, sweep: f64, n: usize| -> Vec<P> {
        let a0 = (from[1] - c[1]).atan2(from[0] - c[0]);
        let r = len(sub(from, c));
        (1..=n).map(|k| {
            let a = a0 + sweep * k as f64 / n as f64;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        }).collect()
    };
    if d.angle == 0.0 {
        let start_end = bay.corners.iter().map(|c| dot(sub(*c, bay.front), t)).fold(f64::MAX, f64::min);
        let rear_end = add(bay.front, mul(t, start_end));
        let toward_aisle = mul(nrm, -s);
        let fin = add(add(rear_end, mul(t, 0.3 + rear_overhang)), mul(toward_aisle, -d.width / 2.0));
        let dy = d.width / 2.0 + 0.5 + wv / 2.0;
        // Both arcs a little wider than full lock, as a driver parks.
        let r = (r_min * 1.05).max(dy / 2.0 + 0.1);
        if dy >= 2.0 * r {
            return None;
        }
        let phi = (1.0 - dy / (2.0 * r)).acos();
        let lx = 2.0 * r * phi.sin();
        let y = |x: f64| -> f64 {
            if x <= lx / 2.0 {
                r - (r * r - x * x).max(0.0).sqrt()
            } else {
                dy - r + (r * r - (x - lx).powi(2)).max(0.0).sqrt()
            }
        };
        // The S ends half a metre short, then straight back: the vehicle
        // ends square in its bay.
        let at = |x: f64, yy: f64| add(add(fin, mul(t, x + 0.5)), mul(toward_aisle, yy));
        let mut path = vec![at(lx + 3.0, dy)];
        let n = 60;
        for k in 0..=n {
            let x = lx * (1.0 - k as f64 / n as f64);
            path.push(at(x, y(x)));
        }
        path.push(fin);
        return Some((path, true));
    }
    let al = d.angle.to_radians();
    let side_len = d.depth / al.sin();
    if d.angle > 89.9 {
        // Reverse in: the front swings out over the aisle; the largest
        // radius keeps its outer corner 0.1 m off the far side.
        let room = d.aisle - 0.1;
        let r = (((room * room - reach * reach).max(0.0)).sqrt() - wv / 2.0).max(r_min);
        let c = add(bay.front, mul(t, r));
        let p1 = sub(c, mul(nrm, s * r));
        let mut path = vec![add(p1, mul(t, 3.0)), p1];
        path.extend(arc(c, p1, -s * std::f64::consts::FRAC_PI_2, 30));
        // Nose out: the rear end 0.3 m off the back of the bay.
        let depth = (side_len - 0.3 - rear_overhang).max(0.3);
        path.push(add(bay.front, mul(bay.into, depth)));
        return Some((path, true));
    }
    // Forward in on an arc from the far side of the aisle.
    let lateral = (d.aisle - wv / 2.0 - 0.25).max(0.3);
    let r = (lateral / (1.0 - al.cos())).min(15.0).max(r_min);
    let c = add(bay.front, mul(left(bay.into), s * r));
    let p1 = sub(c, mul(left(t), s * r));
    let mut path = vec![sub(p1, mul(t, 6.0)), p1];
    path.extend(arc(c, p1, s * al, 30));
    // Nose in: the front 0.3 m off the slanted back of the bay at its
    // nearer corner.
    let depth = (side_len - 0.3 - reach - wv / 2.0 / al.tan()).max(0.3);
    path.push(add(bay.front, mul(bay.into, depth)));
    Some((path, false))
}

pub fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

/// The result of checking one bay with a swept path.
#[derive(Clone, Debug)]
pub struct Check {
    pub bay: usize,
    /// Deepest overlap of the vehicle with a vehicle parked the same way in
    /// another bay (m), and in which bay.
    pub intrusion: f64,
    pub worst_bay: Option<usize>,
    /// How far the vehicle crosses into other bays (their lines), when parked
    /// vehicles leave room.
    pub over_lines: f64,
    /// The vehicle ends inside its bay.
    pub ends_inside: bool,
    pub swept: super::swept::Swept,
}

/// Drive `v` into bay `number` of `lay` and measure what it touches.
pub fn check(v: &Vehicle, d: &Dims, lay: &Layout, number: usize) -> Result<Check, String> {
    let bay = lay.bays.iter().find(|b| b.number == number).ok_or_else(|| t!("geen vak {number}", "no bay {number}"))?;
    let (path, reverse) = entry_path(v, d, bay).ok_or_else(|| t!("{} past niet in de rijbaan", "{} does not fit the aisle", v.name()))?;
    let swept = super::swept::drive_rear(v, &path, 0.05, 0.0, reverse)?;
    let outlines = super::swept::outlines(v, &swept);
    let n_units = v.units.len();
    let parked: Vec<[P; 4]> = outlines[outlines.len() - n_units..].iter().map(|q| ccw(*q)).collect();
    // The same vehicle parked the same way in every other bay: the final
    // outline carried over in each bay's own frame (front edge, along, in);
    // in parallel bays the others stand in the middle of theirs.
    let centre = if d.angle == 0.0 { ((d.length - v.length()) / 2.0 - 0.3).max(0.0) } else { 0.0 };
    let parked_others: Vec<[P; 4]> = parked.iter().map(|q| q.map(|p| add(p, mul(bay.along, centre)))).collect();
    let frame = |b: &Bay, p: P| -> P {
        let det = cross(b.along, b.into);
        let r = sub(p, b.front);
        [cross(r, b.into) / det, cross(b.along, r) / det]
    };
    let place = |b: &Bay, ab: P| add(b.front, add(mul(b.along, ab[0]), mul(b.into, ab[1])));
    let mut intrusion = 0.0f64;
    let mut worst = None;
    let mut over_lines = 0.0f64;
    for other in lay.bays.iter().filter(|b| b.number != number) {
        let cars: Vec<[P; 4]> = parked_others.iter().map(|q| ccw(q.map(|p| place(other, frame(bay, p))))).collect();
        for q in &outlines {
            let q = ccw(*q);
            let deep = cars.iter().map(|c| overlap(&q, c)).fold(0.0, f64::max);
            if deep > intrusion {
                intrusion = deep;
                worst = Some(other.number);
            }
            over_lines = over_lines.max(overlap(&q, &other.corners));
        }
    }
    let ends_inside = parked.iter().all(|q| q.iter().all(|p| inside_depth(*p, &bay.corners) > -0.05));
    Ok(Check { bay: number, intrusion, worst_bay: worst, over_lines, ends_inside, swept })
}

// ── In the drawing ──────────────────────────────────────────────────────────

/// The layers of the parking bays, their numbers and the aisle edges.
pub fn layers() -> (&'static str, &'static str, &'static str) {
    (tr("OI-PARKEREN", "OI-PARKING"), tr("OI-PARKEREN-TEKST", "OI-PARKING-TEXT"), tr("OI-PARKEREN-RIJBAAN", "OI-PARKING-AISLE"))
}

#[derive(Clone, Debug)]
pub struct Request {
    pub class: Class,
    pub angle: f64,
    pub width: Option<f64>,
    pub sides: Sides,
    pub first: usize,
    /// Check one bay with this vehicle's swept path; which bay (default the
    /// middle one of the first row).
    pub vehicle: Option<&'static Vehicle>,
    pub bay: Option<usize>,
}

/// Lay out and draw parking bays along `line` (drawing units).
pub fn draw(line: &[P], req: &Request) -> Result<String, String> {
    let tgt = bridge::target().ok_or_else(|| tr("geen tekening actief", "no drawing active").to_string())?;
    let u = tgt.unit_factor;
    let d = dimensions(req.class, req.angle, req.width)?;
    let metres: Vec<P> = line.iter().map(|p| [p[0] / u, p[1] / u]).collect();
    let lay = layout(&metres, &d, req.sides, req.first);
    if lay.bays.is_empty() {
        return Err(tr("de lijn is te kort voor één parkeervak", "the line is too short for one parking bay").into());
    }
    let (bay_l, text_l, aisle_l) = layers();
    bridge::ensure_layer(bay_l, Color::from_index(7));
    bridge::ensure_layer(text_l, Color::from_index(7));
    bridge::ensure_layer(aisle_l, Color::from_index(8));
    let kind = if d.angle == 0.0 { tr("langs", "parallel").to_string() } else { format!("{:.0}°", d.angle) };
    let mut out: Vec<EntityType> = Vec::new();
    for bay in &lay.bays {
        let mut lw = LwPolyline::new();
        for c in bay.corners {
            lw.add_point_with_bulge(Vector2::new(c[0] * u, c[1] * u), 0.0);
        }
        lw.is_closed = true;
        let mut e = EntityType::LwPolyline(lw);
        let cm = e.common_mut();
        cm.layer = bay_l.into();
        cm.color = Color::ByLayer;
        cm.extended_data.add_record(bridge::object_data([
            ("vak", bay.number.to_string()),
            ("hoek", kind.clone()),
            ("breedte_m", format!("{:.2}", d.width)),
            ("diepte_m", format!("{:.2}", if d.angle == 0.0 { d.length } else { d.depth })),
            ("rijbaan_m", format!("{:.2}", d.aisle)),
            ("klasse", req.class.label().to_string()),
            ("bron", d.source.clone()),
        ]));
        out.push(e);
    }
    for bay in &lay.bays {
        let c = bay.corners.iter().fold([0.0, 0.0], |s, p| add(s, mul(*p, 0.25)));
        let at = Vector3::new(c[0] * u, c[1] * u, 0.0);
        let mut tx = Text::with_value(bay.number.to_string(), at).with_height(0.5 * u);
        tx.horizontal_alignment = TextHorizontalAlignment::Center;
        tx.vertical_alignment = TextVerticalAlignment::Middle;
        tx.alignment_point = Some(at);
        // Read along the bay.
        let ang = bay.into[1].atan2(bay.into[0]);
        tx.rotation = crate::emit::readable(ang);
        let mut e = EntityType::Text(tx);
        e.common_mut().layer = text_l.into();
        out.push(e);
    }
    for edge in &lay.edges {
        let mut lw = LwPolyline::new();
        for c in edge {
            lw.add_point_with_bulge(Vector2::new(c[0] * u, c[1] * u), 0.0);
        }
        let mut e = EntityType::LwPolyline(lw);
        e.common_mut().layer = aisle_l.into();
        out.push(e);
    }
    let mut msg = t!(
        "{} parkeervakken {kind} ({}): vak {:.2} × {:.2} m, rijbaan {:.2} m ({})",
        "{} parking bays {kind} ({}): bay {:.2} × {:.2} m, aisle {:.2} m ({})",
        lay.bays.len(),
        req.class.label(),
        d.width,
        if d.angle == 0.0 { d.length } else { d.depth },
        d.aisle,
        d.source
    );
    crate::civiel::api::note("vakken", lay.bays.len());
    crate::civiel::api::note("vakbreedte_m", d.width);
    crate::civiel::api::note("diepte_m", if d.angle == 0.0 { d.length } else { d.depth });
    crate::civiel::api::note("rijbaan_m", d.aisle);
    crate::civiel::api::note("hoek", d.angle);
    crate::civiel::api::note("bron", d.source.clone());
    crate::civiel::api::note("lagen", serde_json::json!([bay_l, text_l, aisle_l]));
    if let Some(v) = req.vehicle {
        let first_row: Vec<&Bay> = lay.bays.iter().filter(|b| b.side == lay.bays[0].side).collect();
        let number = req.bay.unwrap_or(first_row[first_row.len() / 2].number);
        let c = check(v, &d, &lay, number)?;
        let data = bridge::object_data([
            ("vehicle", v.id.clone()),
            ("vak", number.to_string()),
            ("indringing_m", format!("{:.2}", c.intrusion)),
            ("reverse", if c.swept.reverse { "yes".into() } else { String::new() }),
        ]);
        out.extend(super::place::swept_entities(v, &c.swept, u, &data)?);
        let fits = c.intrusion <= 0.01 && c.ends_inside;
        msg.push_str(&if fits {
            t!(" — vak {number} met {}: past, {:.1}° stuurhoek", " — bay {number} with {}: fits, {:.1}° steering", v.name(), c.swept.max_steer_deg)
        } else {
            let worst = c.worst_bay.map(|w| w.to_string()).unwrap_or_else(|| "-".into());
            t!(
                " — vak {number} met {}: raakt de auto in vak {worst} ({:.2} m), eindstand {}",
                " — bay {number} with {}: touches the vehicle in bay {worst} ({:.2} m), final position {}",
                v.name(),
                c.intrusion,
                if c.ends_inside { tr("binnen het vak", "inside the bay") } else { tr("buiten het vak", "outside the bay") }
            )
        });
        crate::civiel::api::note(
            "controle",
            serde_json::json!({"vak": number, "voertuig": v.id, "indringing_m": (c.intrusion * 100.0).round() / 100.0, "raakt_vak": c.worst_bay, "over_vaklijnen_m": (c.over_lines * 100.0).round() / 100.0, "eindstand_binnen": c.ends_inside, "past": fits, "achteruit": c.swept.reverse, "stuurhoek_max_graden": (c.swept.max_steer_deg * 10.0).round() / 10.0}),
        );
    }
    let first_data = out.first().and_then(|e| e.common().extended_data.get_record(bridge::XDATA_APP).cloned());
    let handles = bridge::add_entities_handles(tr("Parkeervakken", "Parking bays"), out)?;
    if let (Some(h), Some(r)) = (handles.first(), first_data) {
        bridge::register_object_data(*h, r);
    }
    crate::civiel::api::note("elementen", handles.len());
    Ok(msg + ".")
}

/// `GIS_PARKEERVAKKEN`: pick the aisle's centre line, Enter lays out the bays.
pub struct ParkingCmd {
    pub req: Request,
    pub path: Vec<P>,
}

impl ParkingCmd {
    fn run(&self) -> CommandStep {
        let (path, req) = (self.path.clone(), self.req.clone());
        std::thread::spawn(move || match draw(&path, &req) {
            Ok(m) => bridge::info(&m),
            Err(e) => bridge::error(&format!("{}: {e}", tr("Parkeervakken", "Parking bays"))),
        });
        CommandStep::Done
    }
}

impl InteractiveCommand for ParkingCmd {
    fn prompt(&self) -> String {
        if self.path.is_empty() {
            tr("Parkeervakken: begin van de as van de rijbaan", "Parking bays: start of the aisle's centre line").into()
        } else {
            tr("Parkeervakken: volgend punt [Enter = klaar]", "Parking bays: next point [Enter = done]").into()
        }
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        self.path.push([pt[0], pt[1]]);
        CommandStep::NeedPoint
    }
    fn on_enter(&mut self) -> CommandStep {
        if self.path.len() >= 2 {
            self.run()
        } else {
            CommandStep::Cancel
        }
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        if self.path.is_empty() {
            return Vec::new();
        }
        let mut pts: Vec<[f64; 3]> = self.path.iter().map(|p| [p[0], p[1], 0.0]).collect();
        pts.push(pt);
        vec![PreviewWire::with_color(pts, [0.4, 0.8, 1.0, 1.0])]
    }
}

/// `GIS_PARKEERVAKKEN | GIS_PARKING [HOEK=90|80|70|65|60|45|30|LANGS]
/// [KLASSE=openbaar|intensief|nietopenbaar|stalling] [BREEDTE=2.50]
/// [ZIJDE=beide|links|rechts] [LIJN=x,y;x,y] [MET_VEEGPAD=<voertuig>] [VAK=n]
/// [START=1]`: along `LIJN=` (project coordinates), the selected line, or
/// picked points.
pub fn command(host: &mut dyn HostApi, cmd: &str) {
    let mut req = Request { class: Class::Public, angle: 90.0, width: None, sides: Sides::Both, first: 1, vehicle: None, bay: None };
    let mut line_arg = None;
    let mut bad: Option<String> = None;
    let num = |v: &str| v.replace(',', ".").parse::<f64>().ok();
    for w in cmd.split_whitespace().skip(1) {
        let Some((k, v)) = w.split_once('=') else {
            if !(w.contains(',') && w.split(',').all(|n| n.trim().parse::<f64>().is_ok())) {
                bad = Some(t!("onbekend woord '{w}'", "unknown word '{w}'"));
            }
            continue;
        };
        match k.to_ascii_uppercase().as_str() {
            "HOEK" | "ANGLE" => match v.to_ascii_uppercase().as_str() {
                "LANGS" | "PARALLEL" | "0" => req.angle = 0.0,
                "HAAKS" | "PERPENDICULAR" => req.angle = 90.0,
                a => match num(a) {
                    Some(x) => req.angle = x,
                    None => bad = Some(t!("HOEK: 90, 80, 70, 65, 60, 45, 30 of LANGS", "ANGLE: 90, 80, 70, 65, 60, 45, 30 or LANGS")),
                },
            },
            "KLASSE" | "CLASS" => match Class::parse(v) {
                Some(c) => req.class = c,
                None => bad = Some(t!("KLASSE: openbaar, intensief, nietopenbaar of stalling", "CLASS: openbaar (public), intensief (intensive), nietopenbaar (non-public) or stalling (storage)")),
            },
            "BREEDTE" | "WIDTH" => req.width = num(v),
            "ZIJDE" | "SIDE" => {
                req.sides = match v.to_ascii_lowercase().as_str() {
                    "beide" | "both" => Sides::Both,
                    "links" | "left" => Sides::Left,
                    "rechts" | "right" => Sides::Right,
                    _ => {
                        bad = Some(t!("ZIJDE: beide, links of rechts", "SIDE: both, left or right"));
                        Sides::Both
                    }
                }
            }
            "LIJN" | "LINE" => line_arg = Some(v.to_string()),
            "START" => req.first = num(v).map(|x| x.max(0.0) as usize).unwrap_or(1),
            "VAK" | "BAY" => req.bay = num(v).map(|x| x.max(0.0) as usize),
            "MET_VEEGPAD" | "WITH_SWEPT_PATH" | "VEEGPAD" => match super::find(v).or_else(|| matches!(v.to_ascii_lowercase().as_str(), "ja" | "yes").then(|| super::find("car_vw_golf")).flatten()) {
                Some(veh) => req.vehicle = Some(veh),
                None => bad = Some(t!("onbekend voertuig {v} (zie GIS_VOERTUIGEN)", "unknown vehicle {v} (see GIS_VEHICLES)")),
            },
            other => bad = Some(t!("onbekend argument {other}=", "unknown argument {other}=")),
        }
    }
    if let Some(e) = bad.or_else(|| dimensions(req.class, req.angle, req.width).err()) {
        host.push_error(&format!("[Open Infra] {}: {e}", tr("Parkeervakken", "Parking bays")));
        bridge::reply(false, &e);
        return;
    }
    let mut pc = ParkingCmd { req, path: Vec::new() };
    if let Some(text) = line_arg {
        match super::place::project_points(&text) {
            Ok(p) if p.len() >= 2 => {
                pc.path = p;
                let _ = pc.run();
            }
            Ok(_) => {
                host.push_error(&format!("[Open Infra] {}", tr("LIJN= heeft minstens twee punten nodig", "LINE= needs at least two points")));
                bridge::reply(false, "LIJN");
            }
            Err(e) => {
                host.push_error(&format!("[Open Infra] {e}"));
                bridge::reply(false, &e);
            }
        }
        return;
    }
    let sel = host.selection();
    if sel.len() == 1 {
        std::thread::spawn(move || match super::place::selected_line(&sel) {
            Ok(p) => {
                pc.path = p;
                let _ = pc.run();
            }
            Err(e) => bridge::error(&format!("{}: {e}", tr("Parkeervakken", "Parking bays"))),
        });
        return;
    }
    host.start_interactive(Box::new(pc));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimensions_follow_the_tables() {
        let d = dimensions(Class::Public, 90.0, None).unwrap();
        assert_eq!((d.width, d.depth, d.aisle), (2.50, 5.13, 6.00));
        let d = dimensions(Class::Public, 90.0, Some(2.40)).unwrap();
        assert_eq!(d.aisle, 6.67);
        let d = dimensions(Class::NonPublic, 60.0, Some(2.30)).unwrap();
        assert_eq!((d.depth, d.aisle), (5.38, 3.70));
        let d = dimensions(Class::Storage, 45.0, None).unwrap();
        assert_eq!((d.depth, d.aisle), (4.95, 2.75));
        let d = dimensions(Class::Public, 0.0, None).unwrap();
        assert_eq!((d.width, d.length, d.aisle), (2.0, 6.25, 4.0));
        assert_eq!(dimensions(Class::Storage, 0.0, None).unwrap().length, 5.75);
        // Narrower than the class allows, an angle or width not in the table.
        assert!(dimensions(Class::Intensive, 90.0, Some(2.45)).is_err());
        assert!(dimensions(Class::Public, 50.0, None).is_err());
        assert!(dimensions(Class::Public, 90.0, Some(2.42)).is_err());
        assert!(dimensions(Class::Public, 0.0, Some(1.7)).is_err());
        // Every table: P1 + W is the length of the parking unit B1 within a
        // centimetre of the standard's own column (spot checks).
        for (cls, angle, w, b1) in [(Class::Public, 30.0, 2.50, 7.99), (Class::Public, 70.0, 2.40, 9.89), (Class::NonPublic, 80.0, 2.30, 11.24), (Class::Storage, 90.0, 2.50, 10.42)] {
            let d = dimensions(cls, angle, Some(w)).unwrap();
            assert!((d.depth + d.aisle - b1).abs() < 0.011, "{cls:?} {angle} {w}: {}", d.depth + d.aisle);
        }
    }

    #[test]
    fn rows_of_square_angled_and_parallel_bays() {
        let line = [[0.0, 0.0], [50.0, 0.0]];
        // 90°: 50 / 2.50 = 20 bays a side, 40 in all, numbered left then right.
        let d = dimensions(Class::Public, 90.0, None).unwrap();
        let l = layout(&line, &d, Sides::Both, 1);
        assert_eq!(l.bays.len(), 40);
        assert_eq!(l.bays[0].number, 1);
        assert_eq!(l.bays[39].number, 40);
        let b = &l.bays[0];
        assert!((b.corners[0][1] - 3.0).abs() < 1e-9 && (b.corners[2][1] - 8.13).abs() < 1e-9, "{:?}", b.corners);
        // No two bays overlap.
        for (i, a) in l.bays.iter().enumerate() {
            for c in &l.bays[i + 1..] {
                assert!(overlap(&a.corners, &c.corners) < 1e-6);
            }
        }
        // 45°: bays 2.50 / sin 45° = 3.54 m along the aisle, a row 4.95 deep.
        let d = dimensions(Class::Public, 45.0, None).unwrap();
        let l = layout(&line, &d, Sides::Left, 1);
        let n = ((50.0 - 4.95) / (2.5 / 45f64.to_radians().sin())).floor() as usize;
        assert_eq!(l.bays.len(), n);
        let b = &l.bays[0];
        let depth = b.corners.iter().map(|c| c[1]).fold(f64::MIN, f64::max) - 1.9;
        assert!((depth - 4.95).abs() < 1e-9, "{depth}");
        // Bay width square to its sides: 2.50.
        let side = unit(sub(b.corners[3], b.corners[0]));
        let w = cross(side, sub(b.corners[1], b.corners[0])).abs();
        assert!((w - 2.5).abs() < 1e-9, "{w}");
        // Parallel: 50 / 6.25 = 8 bays a side.
        let d = dimensions(Class::Public, 0.0, None).unwrap();
        assert_eq!(layout(&line, &d, Sides::Both, 1).bays.len(), 16);
    }

    #[test]
    fn a_bend_drops_overlapping_bays() {
        let d = dimensions(Class::Public, 90.0, None).unwrap();
        let l = layout(&[[0.0, 0.0], [30.0, 0.0], [30.0, 30.0]], &d, Sides::Both, 1);
        for (i, a) in l.bays.iter().enumerate() {
            for c in &l.bays[i + 1..] {
                assert!(overlap(&a.corners, &c.corners) <= 0.01);
            }
        }
        assert!(l.bays.len() < 2 * 2 * 12);
    }

    #[test]
    fn a_car_fits_a_square_bay_and_a_parallel_one() {
        let car = crate::library::find("car_vw_golf").unwrap();
        let line = [[0.0, 0.0], [40.0, 0.0]];
        let d = dimensions(Class::Public, 90.0, None).unwrap();
        let l = layout(&line, &d, Sides::Both, 1);
        let c = check(car, &d, &l, 8).unwrap();
        assert!(c.ends_inside, "the car ends in its bay");
        assert!(c.intrusion < 0.01, "intrusion {} into bay {:?}", c.intrusion, c.worst_bay);
        // Parallel: a reversing S, rear axle onto the bay's centre line.
        let d = dimensions(Class::Public, 0.0, None).unwrap();
        let l = layout(&line, &d, Sides::Left, 1);
        let c = check(car, &d, &l, 3).unwrap();
        assert!(c.swept.reverse);
        assert!(c.ends_inside);
        assert!(c.intrusion < 0.01, "intrusion {} into bay {:?}", c.intrusion, c.worst_bay);
        // A long van into the 90° bay of a narrow (storage) aisle does not
        // fit without touching its neighbours.
        let van = crate::library::find("van_iveco_daily_l4h3").unwrap();
        let d = dimensions(Class::Storage, 90.0, Some(2.30)).unwrap();
        let l = layout(&line, &d, Sides::Both, 1);
        let c = check(van, &d, &l, 8).unwrap();
        assert!(c.intrusion > 0.05 || !c.ends_inside, "{} {}", c.intrusion, c.ends_inside);
    }
}
