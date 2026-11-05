//! A corridor's design surface as road parts: the pavement (IfcCourse in an
//! IfcPavement), the shoulders and the slopes (IfcCourse PROTECTION), and
//! the earthworks between the design and the existing ground as closed
//! bodies (IfcEarthworksFill and IfcEarthworksCut). Pure geometry in local
//! metres; `mod.rs` writes the entities.

use std::collections::HashMap;

pub type P3 = [f64; 3];

/// Which part of a corridor a design triangle belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Pavement,
    Shoulder,
    Crest,
    Slope,
}

/// The outer edges of the assembly's parts (m from the axis), from the
/// corridor's feature lines; `None` where the assembly has no such part.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Edges {
    pub pavement: Option<f64>,
    pub shoulder: Option<f64>,
    pub crest: Option<f64>,
}

impl Edges {
    /// Widen an edge to a feature line at `offset` (either side).
    pub fn add(&mut self, layer: &str, offset: f64) {
        let l = crate::nlcs::as_nlcs(layer).to_ascii_uppercase();
        let slot = if l.contains("VERHARDING") {
            &mut self.pavement
        } else if l.contains("GRAS_BERM") {
            &mut self.shoulder
        } else if l.contains("DIJK_KRUIN") {
            &mut self.crest
        } else {
            return;
        };
        *slot = Some(slot.map_or(offset.abs(), |e| e.max(offset.abs())));
    }

    /// The part at `offset` (m, either side) from the axis: lanes up to the
    /// pavement edge, then the shoulder (or a dike's crest), then the slopes.
    pub fn part(&self, offset: f64) -> Part {
        let o = offset.abs();
        let within = |e: Option<f64>| e.is_some_and(|e| o <= e + 0.01);
        if within(self.pavement) {
            Part::Pavement
        } else if within(self.shoulder) {
            Part::Shoulder
        } else if within(self.crest) {
            Part::Crest
        } else {
            Part::Slope
        }
    }
}

/// Points shared by position (0.1 mm).
#[derive(Default)]
struct Points {
    points: Vec<P3>,
    index: HashMap<(i64, i64, i64), usize>,
}

impl Points {
    fn at(&mut self, p: P3) -> usize {
        let key = ((p[0] * 1e4).round() as i64, (p[1] * 1e4).round() as i64, (p[2] * 1e4).round() as i64);
        *self.index.entry(key).or_insert_with(|| {
            self.points.push(p);
            self.points.len() - 1
        })
    }
}

/// The triangles of one or more parts, with their own point list.
pub fn part_mesh(verts: &[P3], tris: &[[usize; 3]], parts: &[Part], want: &[Part]) -> (Vec<P3>, Vec<[usize; 3]>) {
    let mut pts = Points::default();
    let mut out = Vec::new();
    for (t, p) in tris.iter().zip(parts) {
        if want.contains(p) {
            out.push(t.map(|i| pts.at(verts[i])));
        }
    }
    (pts.points, out)
}

/// A triangulated body: counter-clockwise faces seen from outside.
#[derive(Clone, Debug, Default)]
pub struct Body {
    pub points: Vec<P3>,
    pub triangles: Vec<[usize; 3]>,
    /// Watertight: every edge used as often in one direction as in the other.
    pub closed: bool,
    /// Enclosed volume (m³), when closed.
    pub volume: f64,
}

fn finish(points: Vec<P3>, triangles: Vec<[usize; 3]>) -> Body {
    let mut directed: HashMap<(usize, usize), usize> = HashMap::new();
    for t in &triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *directed.entry((a, b)).or_default() += 1;
        }
    }
    // Two parts of a body may touch along a line where the thickness is
    // zero (four faces on one edge); that still encloses the volume.
    let closed = !triangles.is_empty() && directed.iter().all(|(&(a, b), &n)| directed.get(&(b, a)) == Some(&n));
    let volume = if closed {
        let o = points[triangles[0][0]];
        let r = |p: P3| [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
        triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| r(points[i]));
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
            })
            .sum()
    } else {
        0.0
    };
    Body { points, triangles, closed, volume }
}

/// Fill and cut on the common subdivision of both TINs. The design is in
/// local IFC metres, the ground in drawing units. Keep every terrain edge
/// before interpolating thickness, including ground inside a design face.
pub(super) fn earth_bodies(points: &[P3], triangles: &[[usize; 3]], ground: &crate::civiel::terrein::Tin, fr: super::Frame) -> [Body; 2] {
    use crate::civiel::terrein::{overlay, Plane, Tin};
    let faces: Vec<_> = triangles.iter().map(|t| t.map(|i| {
        let p = points[i];
        [(p[0] - fr.dx) * fr.f, (p[1] - fr.dy) * fr.f, p[2] * fr.f]
    })).collect();
    let Some(design) = Tin::from_faces(&faces) else { return Default::default() };
    let mut pts = Points::default();
    let mut under = Vec::new();
    let mut tris = Vec::new();
    overlay(&design, ground, |poly, td, tg| {
        let (pd, pg) = (Plane::of(design.corners(td)), Plane::of(ground.corners(tg)));
        let ids: Vec<_> = poly.iter().map(|&xy| {
            let i = pts.at(fr.p(xy[0], xy[1], pd.z(xy)));
            if i == under.len() {
                under.push(Some(pg.z(xy) / fr.f));
            }
            i
        }).collect();
        for k in 1..ids.len() - 1 {
            let t = [ids[0], ids[k], ids[k + 1]];
            if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
                tris.push(t);
            }
        }
    });
    [true, false].map(|fill| earth_body(&pts.points, &under, &tris, fill))
}

/// The earth between a design surface and the ground under it: where the
/// design lies above the ground (`fill`) or below it (cut). Top and bottom
/// are the two surfaces, clipped where they meet (there the body has no
/// thickness); walls close it along the open border of the design surface.
/// `ground` is the height under each design point (None: no ground there,
/// no thickness). Triangles are counter-clockwise seen from above.
pub fn earth_body(points: &[P3], ground: &[Option<f64>], triangles: &[[usize; 3]], fill: bool) -> Body {
    // Thinner than this is no thickness: top and bottom are one point. At
    // least the point grid (0.1 mm), so two heights never merge by rounding.
    const SNAP: f64 = 2e-4;
    let sign = if fill { 1.0 } else { -1.0 };
    // Plan point, signed thickness, top and bottom height.
    type Info = ([f64; 2], f64, f64, f64);
    let info = |i: usize| -> Info {
        let p = points[i];
        let g = ground.get(i).copied().flatten().unwrap_or(p[2]);
        let d = sign * (p[2] - g);
        if d.abs() < SNAP {
            return ([p[0], p[1]], 0.0, p[2], p[2]);
        }
        let (top, bottom) = if fill { (p[2], g) } else { (g, p[2]) };
        ([p[0], p[1]], d, top, bottom)
    };
    let mut uses: HashMap<(usize, usize), usize> = HashMap::new();
    for t in triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut pts = Points::default();
    let mut tris: Vec<[usize; 3]> = Vec::new();
    let push = |t: [usize; 3], tris: &mut Vec<[usize; 3]>| {
        if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
            tris.push(t);
        }
    };
    type Vx = ([f64; 2], f64, f64);
    let corner = |a: &Info| -> Vx { (a.0, a.2, a.3) };
    // Where the thickness is zero on an edge, computed the same way from
    // both triangles that share it (lower point index first).
    let cross = |(i, a): (usize, &Info), (j, b): (usize, &Info)| -> Vx {
        let (a, b) = if i < j { (a, b) } else { (b, a) };
        if a.1 == 0.0 {
            return corner(a);
        }
        if b.1 == 0.0 {
            return corner(b);
        }
        let t = a.1 / (a.1 - b.1);
        let xy = [a.0[0] + (b.0[0] - a.0[0]) * t, a.0[1] + (b.0[1] - a.0[1]) * t];
        let z = a.2 + (b.2 - a.2) * t;
        (xy, z, z)
    };
    for t in triangles {
        let v = t.map(info);
        if v.iter().all(|x| x.1 <= 0.0) {
            continue;
        }
        // Sutherland–Hodgman against thickness > 0.
        let mut poly: Vec<Vx> = Vec::with_capacity(4);
        for k in 0..3 {
            let (a, b) = (&v[k], &v[(k + 1) % 3]);
            if a.1 > 0.0 {
                poly.push(corner(a));
            }
            if (a.1 > 0.0) != (b.1 > 0.0) {
                poly.push(cross((t[k], a), (t[(k + 1) % 3], b)));
            }
        }
        let top: Vec<usize> = poly.iter().map(|p| pts.at([p.0[0], p.0[1], p.1])).collect();
        let bottom: Vec<usize> = poly.iter().map(|p| pts.at([p.0[0], p.0[1], p.2])).collect();
        for k in 1..poly.len().saturating_sub(1) {
            push([top[0], top[k], top[k + 1]], &mut tris);
            push([bottom[0], bottom[k + 1], bottom[k]], &mut tris);
        }
        // Walls on the border: outward is to the right of a counter-clockwise edge.
        for k in 0..3 {
            let (i, j) = (t[k], t[(k + 1) % 3]);
            if uses[&(i.min(j), i.max(j))] != 1 {
                continue;
            }
            let (a, b) = (&v[k], &v[(k + 1) % 3]);
            let (u, w): (Vx, Vx) = match (a.1 > 0.0, b.1 > 0.0) {
                (true, true) => (corner(a), corner(b)),
                (true, false) => (corner(a), cross((i, a), (j, b))),
                (false, true) => (cross((i, a), (j, b)), corner(b)),
                (false, false) => continue,
            };
            let (tu, bu) = (pts.at([u.0[0], u.0[1], u.1]), pts.at([u.0[0], u.0[1], u.2]));
            let (tw, bw) = (pts.at([w.0[0], w.0[1], w.1]), pts.at([w.0[0], w.0[1], w.2]));
            push([tu, bu, bw], &mut tris);
            push([tu, bw, tw], &mut tris);
        }
    }
    finish(pts.points, tris)
}

/// Make every triangle counter-clockwise seen from above.
pub fn upward(verts: &[P3], tris: &mut [[usize; 3]]) {
    for t in tris.iter_mut() {
        let [a, b, c] = t.map(|i| verts[i]);
        if (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]) < 0.0 {
            t.swap(1, 2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_by_offset() {
        let mut e = Edges::default();
        e.add("N-XX-VH-GESLOTENVERHARDING_ASFALT-G", -3.25);
        e.add("N-XX-VH-GESLOTENVERHARDING_ASFALT-G", 3.25);
        e.add("N-XX-GR-GRAS_BERM-G", 4.75);
        e.add("N-XX-GW-TALUD_TEEN-G", 9.0);
        assert_eq!(e, Edges { pavement: Some(3.25), shoulder: Some(4.75), crest: None });
        assert_eq!(e.part(-1.0), Part::Pavement);
        assert_eq!(e.part(4.0), Part::Shoulder);
        assert_eq!(e.part(-6.0), Part::Slope);
        let tris = [[0, 1, 2], [1, 3, 2]];
        let verts = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]];
        let (pts, t) = part_mesh(&verts, &tris, &[Part::Pavement, Part::Slope], &[Part::Slope]);
        assert_eq!((pts.len(), t.len()), (3, 1));
    }

    #[test]
    fn earthworks_are_closed_bodies() {
        // A 20 × 10 m design plane at 1 m over ground at 0, sloping down to
        // the ground on the east side (x = 20): a wedge of 20 × 10 × 1 / 2.
        let pts: Vec<P3> = vec![[0.0, 0.0, 1.0], [20.0, 0.0, 0.0], [20.0, 10.0, 0.0], [0.0, 10.0, 1.0]];
        let tris = vec![[0, 1, 2], [0, 2, 3]];
        let fill = earth_body(&pts, &[Some(0.0); 4], &tris, true);
        assert!(fill.closed, "{:?}", fill.triangles);
        assert!((fill.volume - 100.0).abs() < 1e-9, "{}", fill.volume);
        assert!(earth_body(&pts, &[Some(0.0); 4], &tris, false).triangles.is_empty(), "no cut");
        // Ground at 0.5: fill west of x = 10, cut east of it.
        let fill = earth_body(&pts, &[Some(0.5); 4], &tris, true);
        let cut = earth_body(&pts, &[Some(0.5); 4], &tris, false);
        assert!(fill.closed && cut.closed);
        assert!((fill.volume - 25.0).abs() < 1e-9 && (cut.volume - 25.0).abs() < 1e-9, "{} {}", fill.volume, cut.volume);
        // No ground: no thickness.
        assert!(earth_body(&pts, &[None; 4], &tris, true).triangles.is_empty());
    }

    #[test]
    fn earthworks_preserve_crossing_tin_edges() {
        use crate::civiel::terrein::{volume_between, Tin};
        let points = [[0.0, 0.0, 0.5], [10.0, 0.0, 0.5], [10.0, 10.0, 0.5], [0.0, 10.0, 0.5]];
        let triangles = [[0, 1, 2], [0, 2, 3]];
        let design = Tin::from_faces(&triangles.map(|t| t.map(|i| points[i]))).unwrap();
        // The peak is off the design diagonal: their edges intersect at
        // new points, and some overlay pieces have more than three corners.
        let g = [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 10.0, 0.0], [0.0, 10.0, 0.0], [4.0, 6.0, 1.0]];
        let ground = Tin::from_faces(&[[g[0], g[1], g[4]], [g[1], g[2], g[4]], [g[2], g[3], g[4]], [g[3], g[0], g[4]]]).unwrap();
        let [fill, cut] = earth_bodies(&points, &triangles, &ground, super::super::Frame { f: 1.0, dx: 0.0, dy: 0.0 });
        let expected = volume_between(&ground, &design, 1.0);
        assert!(fill.closed && cut.closed);
        assert!((fill.volume - 125.0 / 6.0).abs() < 1e-9, "{}", fill.volume);
        assert!((cut.volume - 25.0 / 6.0).abs() < 1e-9, "{}", cut.volume);
        assert!((fill.volume - expected.fill).abs() < 1e-9 && (cut.volume - expected.cut).abs() < 1e-9);
    }

    #[test]
    fn earthworks_stop_at_the_ground_boundary() {
        use crate::civiel::terrein::level_tin;
        let points = [[0.0, 0.0, 2.0], [10.0, 0.0, 2.0], [10.0, 10.0, 2.0], [0.0, 10.0, 2.0]];
        let triangles = [[0, 1, 2], [0, 2, 3]];
        // Ground entirely inside the design; none of the design corners
        // have a ground height. Only the common 6 × 6 m area has thickness.
        let ground = level_tin([2.0, 2.0, 8.0, 8.0], 0.0).unwrap();
        let [fill, cut] = earth_bodies(&points, &triangles, &ground, super::super::Frame { f: 1.0, dx: 0.0, dy: 0.0 });
        assert!(fill.closed);
        assert!((fill.volume - 72.0).abs() < 1e-9, "{}", fill.volume);
        assert!(fill.points.iter().all(|p| p[0] >= 2.0 && p[0] <= 8.0 && p[1] >= 2.0 && p[1] <= 8.0));
        assert!(cut.triangles.is_empty());
    }

    #[test]
    fn bodies_touching_along_a_line_stay_watertight() {
        // Two cut wedges on either side of a ridge at ground level (a
        // ditch each side): they touch along the ridge line.
        let mut pts = Vec::new();
        for y in 0..3 {
            for x in 0..3 {
                pts.push([x as f64, y as f64, if y == 1 { 0.00005 } else { -1.0 }]);
            }
        }
        let p = |x: usize, y: usize| y * 3 + x;
        let mut tris = Vec::new();
        for y in 0..2 {
            for x in 0..2 {
                tris.push([p(x, y), p(x + 1, y), p(x + 1, y + 1)]);
                tris.push([p(x, y), p(x + 1, y + 1), p(x, y + 1)]);
            }
        }
        let cut = earth_body(&pts, &[Some(0.0); 9], &tris, false);
        assert!(cut.closed, "{:?}", cut.triangles);
        assert!((cut.volume - 2.0).abs() < 1e-9, "{}", cut.volume);
        assert!(earth_body(&pts, &[Some(0.0); 9], &tris, true).triangles.is_empty(), "0.05 mm is no fill");
        // Clockwise input is turned up first.
        let mut cw = vec![[0usize, 2, 1]];
        upward(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]], &mut cw);
        assert_eq!(cw, vec![[0, 1, 2]]);
    }
}
