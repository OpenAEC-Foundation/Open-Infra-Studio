//! Traverse (polygoonmeting) computation and adjustment, issue #69.
//!
//! A traverse runs from a known start station with a known orientation
//! (backsight) over observed angles and horizontal distances, either back to
//! its start (closed loop, *kringpolygoon*) or to a second known station with
//! a known closing orientation (connecting traverse, *aansluitende polygoon*).
//! The redundancy gives three checks: one angular and two linear (x, y).
//!
//! Conventions (Dutch surveying, RD New):
//! - points are `[x, y]` = `[east, north]` in metres;
//! - azimuths (*richtingshoeken*) are in **gon** (400 gon = full circle),
//!   clockwise from grid north;
//! - angles are in **gon**. A *right* angle (`AngleSense::Right`, the usual
//!   theodolite reading) is measured clockwise from the backsight to the
//!   foresight; a *left* angle counter-clockwise. Then
//!   `t(i→i+1) = t(i−1→i) + β_right − 200` resp. `t(i−1→i) − β_left + 200`.
//!
//! Adjustment follows the classic sequence (e.g. Ghilani & Wolf, *Elementary
//! Surveying*, traverse computations; Ghilani, *Adjustment Computations*, for the
//! least-squares alternative):
//! 1. angular misclosure `w = t_computed − t_known` at the closing direction,
//!    distributed equally over all angles (`−w/n` each);
//! 2. latitudes/departures from the corrected azimuths, linear misclosure
//!    `fx, fy`, `f = √(fx² + fy²)`, relative precision `1 : ΣL/f`;
//! 3. distribution of `fx, fy` by the compass rule (Bowditch: proportional to
//!    the cumulative distance) or the transit rule (proportional to the
//!    cumulative |Δx| resp. |Δy|).
//!
//! Extension point: a least-squares adjustment (weights from the angle and
//! distance standard deviations, three condition equations) belongs in
//! [`Adjustment`] as a third variant; the report struct already carries all
//! quantities it would fill.

use crate::P2;
use serde::{Deserialize, Serialize};

/// Full circle in gon.
pub const FULL_GON: f64 = 400.0;

/// Direction in which the traverse angles were read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AngleSense {
    /// Clockwise from backsight to foresight (*rechtse hoek*).
    Right,
    /// Counter-clockwise from backsight to foresight (*linkse hoek*).
    Left,
}

/// A known orientation: an azimuth or a known point to sight on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Orientation {
    /// Azimuth in gon from the station to the sighted target.
    Azimuth(f64),
    /// Known coordinates of the sighted target.
    Point(P2),
}

/// How the traverse is closed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Closure {
    /// Back to the start station; the closing angle at the start sights the
    /// original backsight again.
    Loop,
    /// To a known end station with a known closing foresight.
    Connecting { end: P2, foresight: Orientation },
}

/// Distribution rule for the linear misclosure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Adjustment {
    /// Compass rule (Bowditch): corrections ∝ cumulative distance.
    Compass,
    /// Transit rule: x-corrections ∝ cumulative |Δx|, y ∝ cumulative |Δy|.
    Transit,
}

/// Observed traverse.
///
/// With `k = distances.len()` legs there are `k + 1` angles: one at the start
/// (backsight → first leg), one at each of the `k − 1` new points and one at
/// the end station (last leg's backsight → closing foresight). For a loop the
/// end station is the start and the closing foresight is the start backsight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Traverse {
    pub start: P2,
    pub backsight: Orientation,
    pub angles_gon: Vec<f64>,
    pub distances: Vec<f64>,
    pub sense: AngleSense,
    pub closure: Closure,
}

/// Result of [`adjust`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraverseReport {
    /// Angular misclosure `computed − known` closing azimuth, gon.
    pub angular_misclosure_gon: f64,
    /// Correction added to every observed angle, gon: `−w/n` for right
    /// angles, `+w/n` for left angles.
    pub angle_correction_gon: f64,
    /// Adjusted leg azimuths, gon.
    pub azimuths_gon: Vec<f64>,
    /// Linear misclosure east (computed − known), m.
    pub fx: f64,
    /// Linear misclosure north (computed − known), m.
    pub fy: f64,
    /// Total linear misclosure, m.
    pub f: f64,
    /// Total traverse length ΣL, m.
    pub length: f64,
    /// Relative precision `n` in 1 : n (infinite for zero misclosure).
    pub relative_precision: f64,
    /// Coordinates after angle correction, before linear adjustment.
    pub unadjusted: Vec<P2>,
    /// Applied coordinate corrections `[dx, dy]` per station.
    pub corrections: Vec<[f64; 2]>,
    /// Adjusted coordinates, start … end inclusive.
    pub points: Vec<P2>,
    pub method: Adjustment,
}

/// Azimuth from `a` to `b` in gon, in [0, 400).
pub fn azimuth_gon(a: P2, b: P2) -> f64 {
    let t = (b[0] - a[0]).atan2(b[1] - a[1]) * 200.0 / std::f64::consts::PI;
    norm_gon(t)
}

/// Point at azimuth `t_gon` and horizontal distance `d` from `p`.
pub fn polar(p: P2, t_gon: f64, d: f64) -> P2 {
    let t = t_gon * std::f64::consts::PI / 200.0;
    [p[0] + d * t.sin(), p[1] + d * t.cos()]
}

/// Normalise to [0, 400) gon.
pub fn norm_gon(t: f64) -> f64 {
    let r = t.rem_euclid(FULL_GON);
    if r >= FULL_GON {
        0.0
    } else {
        r
    }
}

/// Normalise a difference to (−200, 200] gon.
pub fn diff_gon(t: f64) -> f64 {
    let r = norm_gon(t);
    if r > 200.0 {
        r - FULL_GON
    } else {
        r
    }
}

/// Angle at a station in the requested sense from the backsight and
/// foresight azimuths (handy to build test data or check field books).
pub fn angle_gon(backsight_az: f64, foresight_az: f64, sense: AngleSense) -> f64 {
    match sense {
        AngleSense::Right => norm_gon(foresight_az - backsight_az),
        AngleSense::Left => norm_gon(backsight_az - foresight_az),
    }
}

fn orientation_az(from: P2, o: &Orientation) -> f64 {
    match o {
        Orientation::Azimuth(t) => norm_gon(*t),
        Orientation::Point(p) => azimuth_gon(from, *p),
    }
}

/// Carry an azimuth over one station: incoming leg azimuth `t_in` (towards
/// the station) and the observed angle give the outgoing azimuth.
fn carry(t_in: f64, beta: f64, sense: AngleSense) -> f64 {
    match sense {
        AngleSense::Right => norm_gon(t_in + beta - 200.0),
        AngleSense::Left => norm_gon(t_in - beta + 200.0),
    }
}

/// Compute and adjust a traverse.
pub fn adjust(tr: &Traverse, method: Adjustment) -> Result<TraverseReport, String> {
    let k = tr.distances.len();
    if k == 0 {
        return Err("traverse needs at least one leg".into());
    }
    if tr.angles_gon.len() != k + 1 {
        return Err(format!(
            "expected {} angles for {} legs (start, intermediate, end), got {}",
            k + 1,
            k,
            tr.angles_gon.len()
        ));
    }
    if tr.distances.iter().any(|d| !(d.is_finite() && *d > 0.0)) {
        return Err("distances must be positive".into());
    }
    let back_az = orientation_az(tr.start, &tr.backsight);
    let (end, close_az) = match &tr.closure {
        Closure::Loop => (tr.start, back_az),
        Closure::Connecting { end, foresight } => (*end, orientation_az(*end, foresight)),
    };

    // 1. Angular misclosure. The backsight azimuth seen "from behind" is the
    // incoming azimuth t_in = back_az + 200.
    let n_ang = tr.angles_gon.len() as f64;
    let mut t = norm_gon(back_az + 200.0);
    for &b in &tr.angles_gon {
        t = carry(t, b, tr.sense);
    }
    let w = diff_gon(t - close_az);
    let corr = -w / n_ang;
    // Corrected angles: a correction on a left angle acts the other way.
    let signed_corr = match tr.sense {
        AngleSense::Right => corr,
        AngleSense::Left => -corr,
    };
    let mut az = Vec::with_capacity(k);
    let mut t = norm_gon(back_az + 200.0);
    for &b in &tr.angles_gon[..k] {
        t = carry(t, b + signed_corr, tr.sense);
        az.push(t);
    }

    // 2. Coordinates and linear misclosure.
    let mut pts = vec![tr.start];
    let mut dxs = Vec::with_capacity(k);
    for (i, &d) in tr.distances.iter().enumerate() {
        let p = polar(*pts.last().unwrap(), az[i], d);
        dxs.push([p[0] - pts[i][0], p[1] - pts[i][1]]);
        pts.push(p);
    }
    let last = pts[k];
    let fx = last[0] - end[0];
    let fy = last[1] - end[1];
    let f = fx.hypot(fy);
    let length: f64 = tr.distances.iter().sum();
    let relative_precision = if f > 0.0 { length / f } else { f64::INFINITY };

    // 3. Distribution.
    let (sx, sy): (f64, f64) = match method {
        Adjustment::Compass => (length, length),
        Adjustment::Transit => (
            dxs.iter().map(|d| d[0].abs()).sum(),
            dxs.iter().map(|d| d[1].abs()).sum(),
        ),
    };
    let mut corrections = vec![[0.0, 0.0]];
    let (mut cx, mut cy) = (0.0, 0.0);
    for (i, d) in dxs.iter().enumerate() {
        match method {
            Adjustment::Compass => {
                cx += tr.distances[i];
                cy += tr.distances[i];
            }
            Adjustment::Transit => {
                cx += d[0].abs();
                cy += d[1].abs();
            }
        }
        let kx = if sx > 0.0 { cx / sx } else { 0.0 };
        let ky = if sy > 0.0 { cy / sy } else { 0.0 };
        corrections.push([-fx * kx, -fy * ky]);
    }
    let adj: Vec<P2> = pts
        .iter()
        .zip(&corrections)
        .map(|(p, c)| [p[0] + c[0], p[1] + c[1]])
        .collect();
    let azimuths_gon = (0..k).map(|i| azimuth_gon(adj[i], adj[i + 1])).collect();

    Ok(TraverseReport {
        angular_misclosure_gon: w,
        angle_correction_gon: signed_corr,
        azimuths_gon,
        fx,
        fy,
        f,
        length,
        relative_precision,
        unadjusted: pts,
        corrections,
        points: adj,
        method,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build exact observations (right angles) for a station list.
    fn observe(stations: &[P2], back: P2, close: P2) -> (Vec<f64>, Vec<f64>) {
        let mut angles = Vec::new();
        let mut dist = Vec::new();
        let n = stations.len();
        for i in 0..n {
            let bs = if i == 0 { back } else { stations[i - 1] };
            let fs = if i + 1 < n { stations[i + 1] } else { close };
            angles.push(angle_gon(
                azimuth_gon(stations[i], bs),
                azimuth_gon(stations[i], fs),
                AngleSense::Right,
            ));
            if i + 1 < n {
                dist.push(
                    (stations[i + 1][0] - stations[i][0])
                        .hypot(stations[i + 1][1] - stations[i][1]),
                );
            }
        }
        (angles, dist)
    }

    fn square() -> Traverse {
        // Loop A(0,0) → B(100,0) → C(100,100) → D(0,100) → A, backsight north.
        let st = [
            [0.0, 0.0],
            [100.0, 0.0],
            [100.0, 100.0],
            [0.0, 100.0],
            [0.0, 0.0],
        ];
        let back = [0.0, 50.0];
        let (angles, distances) = observe(&st, back, back);
        Traverse {
            start: st[0],
            backsight: Orientation::Point(back),
            angles_gon: angles,
            distances,
            sense: AngleSense::Right,
            closure: Closure::Loop,
        }
    }

    #[test]
    fn perfect_loop_has_zero_misclosure() {
        let tr = square();
        let r = adjust(&tr, Adjustment::Compass).unwrap();
        assert!(r.angular_misclosure_gon.abs() < 1e-9);
        assert!(r.f < 1e-9);
        assert!(r.relative_precision > 1e9);
        assert!((r.points[2][0] - 100.0).abs() < 1e-9 && (r.points[2][1] - 100.0).abs() < 1e-9);
    }

    #[test]
    fn introduced_angle_error_is_found_and_removed() {
        let mut tr = square();
        tr.angles_gon[2] += 0.0050; // 5 mgon
        let r = adjust(&tr, Adjustment::Compass).unwrap();
        assert!((r.angular_misclosure_gon - 0.0050).abs() < 1e-9);
        assert!((r.angle_correction_gon + 0.0050 / 5.0).abs() < 1e-12);
    }

    #[test]
    fn introduced_distance_error_gives_expected_linear_misclosure() {
        let mut tr = square();
        tr.distances[1] += 0.04; // leg B→C (north) 4 cm too long
        let r = adjust(&tr, Adjustment::Compass).unwrap();
        assert!(r.angular_misclosure_gon.abs() < 1e-9);
        assert!(r.fx.abs() < 1e-9);
        assert!((r.fy - 0.04).abs() < 1e-9);
        assert!((r.relative_precision - 400.04 / 0.04).abs() < 1e-3);
        // Closure: adjusted end equals start.
        let e = r.points[4];
        assert!(e[0].abs() < 1e-9 && e[1].abs() < 1e-9);
    }

    #[test]
    fn bowditch_corrections_proportional_to_cumulative_distance() {
        let mut tr = square();
        tr.distances[0] += 0.03;
        tr.distances[1] -= 0.02;
        let r = adjust(&tr, Adjustment::Compass).unwrap();
        let mut cum = 0.0;
        for i in 1..r.points.len() {
            cum += tr.distances[i - 1];
            let k = cum / r.length;
            assert!((r.corrections[i][0] + r.fx * k).abs() < 1e-12);
            assert!((r.corrections[i][1] + r.fy * k).abs() < 1e-12);
        }
    }

    #[test]
    fn transit_rule_closes_too() {
        let mut tr = square();
        tr.distances[0] += 0.03;
        tr.distances[3] += 0.01;
        let r = adjust(&tr, Adjustment::Transit).unwrap();
        let e = r.points[4];
        assert!(e[0].abs() < 1e-9 && e[1].abs() < 1e-9);
        // Leg A→B has no Δy, so B gets no y-correction.
        assert!(r.corrections[1][1].abs() < 1e-12);
    }

    #[test]
    fn connecting_traverse_with_left_angles() {
        let st = [
            [1000.0, 2000.0],
            [1080.0, 2060.0],
            [1180.0, 2050.0],
            [1250.0, 2120.0],
        ];
        let back = [900.0, 2000.0];
        let close = [1250.0, 2300.0];
        let (right, distances) = observe(&st, back, close);
        let left: Vec<f64> = right.iter().map(|b| norm_gon(400.0 - b)).collect();
        let tr = Traverse {
            start: st[0],
            backsight: Orientation::Point(back),
            angles_gon: left,
            distances,
            sense: AngleSense::Left,
            closure: Closure::Connecting {
                end: st[3],
                foresight: Orientation::Point(close),
            },
        };
        let r = adjust(&tr, Adjustment::Compass).unwrap();
        assert!(r.angular_misclosure_gon.abs() < 1e-9);
        for (p, q) in r.points.iter().zip(&st) {
            assert!((p[0] - q[0]).abs() < 1e-8 && (p[1] - q[1]).abs() < 1e-8);
        }
    }

    #[test]
    fn rejects_wrong_angle_count() {
        let mut tr = square();
        tr.angles_gon.pop();
        assert!(adjust(&tr, Adjustment::Compass).is_err());
    }
}
