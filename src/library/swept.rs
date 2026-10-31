//! Swept paths: a vehicle driven along a path, the way swept-path software
//! draws it, with the low-speed (no side slip) kinematics that swept-path
//! software uses.
//!
//! - **Forward:** the front axle centre follows the path; each rear axle and
//!   each trailer follows as a tractrix.
//! - **Reverse:** the rear axle of the last unit follows the path backwards
//!   (its heading is the path's direction, turned round), and the units ahead
//!   follow from it. For each unit ahead the hitch point is known (it is the
//!   kingpin of the unit behind); its own axle moves only along its heading.
//!   That relation is a tractrix too, but it is stable in one direction of
//!   travel only: with the coupling ahead of the unit's axle (a fifth wheel)
//!   the axle trails the hitch when the vehicle moves forward, so it is
//!   integrated from the end of the path to its start; with the coupling
//!   behind the axle (a drawbar hitch) it trails in the reversing direction
//!   and is integrated from start to end. Both give the same no-slip motion;
//!   only the stable direction converges.
//!
//! The result is the traces of the body corners and the outer wheel edges,
//! the vehicle's position every so often, the largest steering and
//! articulation angles and the tightest outer radius.
//!
//! Coordinates are metres in the plane; the path runs from its first point
//! to its last. The vehicle starts aligned with the first segment.

use super::draw::unit_fronts;
use super::{Unit, Vehicle};

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

/// Where a unit's guiding axle sits (from its front), and the effective
/// rear axle (the middle of the non-steering axles: a tandem acts as one).
pub fn axles(u: &Unit) -> Option<(f64, f64)> {
    let steer: Vec<f64> = u.axles.iter().filter(|a| a.steer).map(|a| a.x).collect();
    let fixed: Vec<f64> = u.axles.iter().filter(|a| !a.steer).map(|a| a.x).collect();
    let rear = if fixed.is_empty() { u.axles.iter().map(|a| a.x).fold(f64::MIN, f64::max) } else { fixed.iter().sum::<f64>() / fixed.len() as f64 };
    let front = steer.first().copied().or_else(|| u.axles.iter().map(|a| a.x).reduce(f64::min))?;
    (rear > front).then_some((front, rear))
}

/// One unit's pose: its front (bumper centre) and its forward direction.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub front: P,
    pub dir: P,
}

impl Pose {
    /// A point `back` metres behind the front and `side` metres to the left.
    pub fn at(&self, back: f64, side: f64) -> P {
        add(sub(self.front, mul(self.dir, back)), mul(left(self.dir), side))
    }
    pub fn angle(&self) -> f64 {
        self.dir[1].atan2(self.dir[0])
    }
}

/// The angle between two units (degrees, 0 when in line).
pub fn articulation(a: &Pose, b: &Pose) -> f64 {
    let c = (a.dir[0] * b.dir[0] + a.dir[1] * b.dir[1]).clamp(-1.0, 1.0);
    c.acos().to_degrees()
}

#[derive(Clone, Debug, Default)]
pub struct Swept {
    /// Traces: body corners (front left/right, rear left/right per unit)
    /// and outer wheel edges, each a polyline.
    pub body: Vec<Vec<P>>,
    pub wheels: Vec<Vec<P>>,
    /// The vehicle's units every `ghost_every` metres: per position, a pose per unit.
    pub ghosts: Vec<Vec<Pose>>,
    /// Every simulated position (a pose per unit), in the order driven.
    pub poses: Vec<Vec<Pose>>,
    /// Largest steering angle the path asks for (degrees).
    pub max_steer_deg: f64,
    /// Largest angle between two coupled units (degrees), and where along
    /// the path it occurs (m).
    pub max_articulation_deg: f64,
    pub articulation_at: f64,
    /// Smallest radius of the outer front corner's trace (m), if the path turns.
    pub min_outer_radius: Option<f64>,
    pub length: f64,
    pub reverse: bool,
}

/// Resample a polyline every `step` metres.
fn resample(path: &[P], step: f64) -> Vec<P> {
    let mut out = vec![path[0]];
    let mut carry = 0.0;
    for w in path.windows(2) {
        let (a, b) = (w[0], w[1]);
        let l = len(sub(b, a));
        if l < 1e-9 {
            continue;
        }
        let d = unit(sub(b, a));
        let mut s = step - carry;
        while s <= l + 1e-9 {
            out.push(add(a, mul(d, s)));
            s += step;
        }
        carry = l - (s - step);
    }
    if let (Some(last), Some(end)) = (out.last().copied(), path.last().copied()) {
        if len(sub(end, last)) > 1e-6 {
            out.push(end);
        }
    }
    out
}

/// Radius of the circle through three points (∞ when straight).
fn radius(a: P, b: P, c: P) -> f64 {
    let (ab, bc, ca) = (len(sub(b, a)), len(sub(c, b)), len(sub(a, c)));
    let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    if cross.abs() < 1e-9 {
        f64::INFINITY
    } else {
        ab * bc * ca / (2.0 * cross.abs())
    }
}

fn geometry(v: &Vehicle) -> Result<Vec<(f64, f64)>, String> {
    v.units.iter().map(|u| axles(u).ok_or_else(|| format!("{}: a unit needs two axles for a swept path", v.id))).collect()
}

/// Traces, ghosts and angles from a series of poses (one per simulation
/// step), `travel[k]` metres along the guide path.
fn assemble(v: &Vehicle, poses: Vec<Vec<Pose>>, travel: &[f64], ghost_every: f64) -> Swept {
    let corners = |p: &Pose, u: &Unit| [p.at(0.0, u.width / 2.0), p.at(0.0, -u.width / 2.0), p.at(u.length, u.width / 2.0), p.at(u.length, -u.width / 2.0)];
    let wheel_keys: Vec<(usize, f64, f64)> = v
        .units
        .iter()
        .enumerate()
        .flat_map(|(i, u)| {
            u.axles.iter().flat_map(move |a| {
                let half = if a.track > 0.05 { a.track / 2.0 } else { (u.width / 2.0 - 0.2).max(0.0) } + if a.dual { 0.33 } else { 0.12 };
                [(i, a.x, half), (i, a.x, -half)]
            })
        })
        .collect();
    let mut body: Vec<Vec<P>> = vec![Vec::with_capacity(poses.len()); v.units.len() * 4];
    let mut wheels: Vec<Vec<P>> = vec![Vec::with_capacity(poses.len()); wheel_keys.len()];
    let mut ghosts = Vec::new();
    let mut next_ghost = 0.0;
    let (mut max_art, mut art_at) = (0.0f64, 0.0);
    for (k, ps) in poses.iter().enumerate() {
        for (i, (p, u)) in ps.iter().zip(&v.units).enumerate() {
            for (c, q) in corners(p, u).into_iter().enumerate() {
                body[i * 4 + c].push(q);
            }
        }
        for (w, (i, x, y)) in wheel_keys.iter().enumerate() {
            wheels[w].push(ps[*i].at(*x, *y));
        }
        for pair in ps.windows(2) {
            let a = articulation(&pair[0], &pair[1]);
            if a > max_art {
                max_art = a;
                art_at = travel[k];
            }
        }
        if k == 0 || (ghost_every > 0.0 && travel[k] >= next_ghost + ghost_every) {
            if k > 0 {
                next_ghost += ghost_every;
            }
            ghosts.push(ps.clone());
        }
    }
    let total = travel.last().copied().unwrap_or(0.0);
    if let Some(last) = poses.last() {
        if ghosts.len() == 1 || ghost_every <= 0.0 || total - next_ghost > 1e-6 {
            ghosts.push(last.clone());
        }
    }
    Swept { body, wheels, ghosts, poses, max_articulation_deg: max_art, articulation_at: art_at, length: total, ..Default::default() }
}

/// Tightest radius of the outer front corner: the corner that swings wide.
fn outer_radius(s: &Swept, m: usize) -> Option<f64> {
    let outer = |t: &Vec<P>| -> Option<f64> {
        let r: Vec<f64> = t.windows(2 * m + 1).step_by(m).map(|w| radius(w[0], w[m], w[2 * m])).filter(|r| r.is_finite()).collect();
        r.into_iter().reduce(f64::min)
    };
    [&s.body[0], &s.body[1]].into_iter().filter_map(outer).reduce(f64::max)
}

/// Drive `v` along `path` (metres). `step` is the simulation step, `ghost_every`
/// the distance between drawn vehicle positions (0: start and end only).
pub fn drive(v: &Vehicle, path: &[P], step: f64, ghost_every: f64) -> Result<Swept, String> {
    if path.len() < 2 {
        return Err("a swept path needs a path of at least two points".into());
    }
    let fronts = unit_fronts(v);
    let geo = geometry(v)?;
    let step = step.max(0.02);
    let pts = resample(path, step);
    // Curvature over about a metre either side: the path's vertices would
    // otherwise read as sharp kinks.
    let m = ((1.0 / step).round() as usize).max(1);
    let d0 = unit(sub(pts[1.min(pts.len() - 1)], pts[0]));
    // Initial poses: everything straight behind the start, along the first segment.
    let mut rear: Vec<P> = Vec::with_capacity(v.units.len());
    let mut poses: Vec<Pose> = Vec::with_capacity(v.units.len());
    let (f0, r0) = geo[0];
    let start_front = add(pts[0], mul(d0, f0));
    for i in 0..v.units.len() {
        let front = sub(start_front, mul(d0, fronts[i]));
        let pose = Pose { front, dir: d0 };
        rear.push(pose.at(if i == 0 { r0 } else { geo[i].1 }, 0.0));
        poses.push(pose);
    }
    let mut all = vec![poses.clone()];
    let mut travel = vec![0.0];
    let mut max_steer: f64 = 0.0;
    let wb0 = r0 - f0;
    for k in 1..pts.len() {
        // The steer axle centre moves along the path; each rear axle follows.
        let fa = pts[k];
        let r_new = add(fa, mul(unit(sub(rear[0], fa)), wb0));
        rear[0] = r_new;
        let d = unit(sub(fa, r_new));
        poses[0] = Pose { front: add(fa, mul(d, f0)), dir: d };
        for i in 1..v.units.len() {
            let prev = poses[i - 1];
            let hitch = prev.at(crate::library::draw::hitch_x(&v.units[i - 1]), 0.0);
            let king = crate::library::draw::kingpin_x(&v.units[i]);
            let (_, rx) = geo[i];
            let wb = rx - king;
            let r_new = add(hitch, mul(unit(sub(rear[i], hitch)), wb.abs().max(0.1)));
            rear[i] = r_new;
            let d = unit(sub(hitch, r_new));
            poses[i] = Pose { front: add(hitch, mul(d, king)), dir: d };
        }
        // Steering angle from the turning circle of the steer axle.
        if k >= m && k + m < pts.len() {
            let rad = radius(pts[k - m], pts[k], pts[k + m]);
            if rad.is_finite() {
                max_steer = max_steer.max((wb0 / rad).atan().to_degrees());
            }
        }
        travel.push(travel[k - 1] + len(sub(pts[k], pts[k - 1])));
        all.push(poses.clone());
    }
    let mut s = assemble(v, all, &travel, ghost_every);
    s.max_steer_deg = max_steer;
    s.min_outer_radius = outer_radius(&s, m);
    Ok(s)
}

/// The no-slip follower of a hitch track `h`: the axle stays `e` metres
/// behind the hitch along the unit (`e` < 0: ahead of it). Integrated in the
/// direction where the axle trails the hitch; `h0` is the unit's heading at
/// the start of that integration. Returns the unit's heading per sample.
fn follow(h: &[P], e: f64, h0: P, motion_reversed: bool) -> Vec<P> {
    let n = h.len();
    let mut dir = vec![h0; n];
    if e.abs() < 0.05 {
        // Coupling over the axle: the unit points along its hitch's track.
        for k in 0..n {
            let (a, b) = (h[k.saturating_sub(1)], h[(k + 1).min(n - 1)]);
            let d = sub(b, a);
            if len(d) > 1e-9 {
                // The track runs in the direction of travel; reversing, the
                // unit points the other way.
                dir[k] = if motion_reversed { mul(unit(d), -1.0) } else { unit(d) };
            } else if k > 0 {
                dir[k] = dir[k - 1];
            }
        }
        return dir;
    }
    // Forward motion of the vehicle is towards decreasing k when reversing.
    let forward_order: Vec<usize> = if motion_reversed { (0..n).rev().collect() } else { (0..n).collect() };
    let order: Vec<usize> = if e > 0.0 { forward_order } else { forward_order.into_iter().rev().collect() };
    let first = order[0];
    // Axle = hitch − e·heading.
    let mut axle = sub(h[first], mul(h0, e));
    for &k in &order {
        axle = add(h[k], mul(unit(sub(axle, h[k])), e.abs()));
        // e > 0: heading from axle to hitch; e < 0: from hitch to axle.
        dir[k] = if e > 0.0 { unit(sub(h[k], axle)) } else { unit(sub(axle, h[k])) };
    }
    dir
}

/// Reverse `v` along `path` (metres): the rear axle of the last unit runs
/// from the first point to the last, backwards; the units ahead follow.
/// Use a smooth path (arcs): a sharp corner turns the last unit over about
/// a metre.
pub fn drive_reverse(v: &Vehicle, path: &[P], step: f64, ghost_every: f64) -> Result<Swept, String> {
    drive_rear(v, path, step, ghost_every, true)
}

/// Drive `v` with the rear axle of its last unit on `path`, forward
/// (`reversing` false: the axle path is given, as for a precise manoeuvre)
/// or backwards.
pub fn drive_rear(v: &Vehicle, path: &[P], step: f64, ghost_every: f64, reversing: bool) -> Result<Swept, String> {
    if path.len() < 2 {
        return Err("a swept path needs a path of at least two points".into());
    }
    let geo = geometry(v)?;
    let step = step.max(0.02);
    let pts = resample(path, step);
    let n = pts.len();
    let m = ((0.5 / step).round() as usize).max(1);
    let last = v.units.len() - 1;
    // The last unit's heading: the path's direction (turned round when
    // reversing), over about a metre (a polyline corner would otherwise turn
    // it on the spot).
    let mut dirs: Vec<Vec<P>> = vec![Vec::new(); v.units.len()];
    dirs[last] = (0..n)
        .map(|k| {
            let (a, b) = (pts[k.saturating_sub(m)], pts[(k + m).min(n - 1)]);
            if reversing { unit(sub(a, b)) } else { unit(sub(b, a)) }
        })
        .collect();
    // Axle tracks: the last unit's is the path itself.
    let mut axle: Vec<Vec<P>> = vec![Vec::new(); v.units.len()];
    axle[last] = pts.clone();
    for i in (0..last).rev() {
        let below = &v.units[i + 1];
        let (_, rb) = geo[i + 1];
        let king = crate::library::draw::kingpin_x(below);
        // The kingpin of the unit behind is the hitch of this one.
        let hitch: Vec<P> = (0..n).map(|k| add(axle[i + 1][k], mul(dirs[i + 1][k], rb - king))).collect();
        let (_, r) = geo[i];
        let e = r - crate::library::draw::hitch_x(&v.units[i]);
        // Start of the stable integration: along the hitch's track, which is
        // where the unit settles anyway (it removes the start-up wobble).
        let from_end = (e > 0.0) == reversing;
        let k0 = if from_end { n - 1 } else { 0 };
        let track = sub(hitch[(k0 + m).min(n - 1)], hitch[k0.saturating_sub(m)]);
        let h0 = if len(track) < 1e-9 {
            dirs[i + 1][k0]
        } else if reversing {
            unit(mul(track, -1.0))
        } else {
            unit(track)
        };
        dirs[i] = follow(&hitch, e, h0, reversing);
        axle[i] = (0..n).map(|k| sub(hitch[k], mul(dirs[i][k], e))).collect();
    }
    let mut travel = vec![0.0; n];
    for k in 1..n {
        travel[k] = travel[k - 1] + len(sub(pts[k], pts[k - 1]));
    }
    let all: Vec<Vec<Pose>> = (0..n).map(|k| (0..v.units.len()).map(|i| Pose { front: add(axle[i][k], mul(dirs[i][k], geo[i].1)), dir: dirs[i][k] }).collect()).collect();
    // Steering of the first unit from the curvature of its axle track
    // (no slip: tan δ = wheelbase / radius of the rear axle).
    let (f0, r0) = geo[0];
    let wb0 = r0 - f0;
    let mut max_steer: f64 = 0.0;
    let mm = ((1.0 / step).round() as usize).max(1);
    for k in mm..n.saturating_sub(mm) {
        let rad = radius(axle[0][k - mm], axle[0][k], axle[0][k + mm]);
        if rad.is_finite() {
            max_steer = max_steer.max((wb0 / rad).atan().to_degrees());
        }
    }
    let mut s = assemble(v, all, &travel, ghost_every);
    s.max_steer_deg = max_steer;
    s.min_outer_radius = outer_radius(&s, mm);
    s.reverse = reversing;
    Ok(s)
}

/// The articulation beyond which a reversing combination can no longer be
/// straightened: the steady articulation with the steered unit at full lock
/// (in a steady turn every axle runs on a circle round one centre; the
/// coupling sits on the kingpin circle, so each unit is tilted against that
/// circle by asin(distance from axle to coupling / kingpin radius)). For a
/// steered unit with one trailer; full lock comes from the vehicle's outer
/// turning radius (the outer front corner), so the angle is only as good as
/// that radius and the assumption that it belongs to the steered unit.
pub fn jackknife_angle(v: &Vehicle) -> Option<f64> {
    if v.units.len() != 2 {
        return None;
    }
    let r_out = v.turning_radius_outer?;
    let (_, r0) = axles(&v.units[0])?;
    let (_, r1) = axles(&v.units[1])?;
    // Rear axle radius at full lock: the outer front corner is r0 ahead of
    // the axle and half a width outside it.
    let ra = (r_out * r_out - r0 * r0).sqrt() - v.units[0].width / 2.0;
    if !ra.is_finite() || ra <= 0.0 {
        return None;
    }
    let e = r0 - crate::library::draw::hitch_x(&v.units[0]);
    let wb_t = r1 - crate::library::draw::kingpin_x(&v.units[1]);
    let rk = (ra * ra + e * e).sqrt();
    if wb_t >= rk {
        return Some(90.0);
    }
    Some(((wb_t / rk).asin() - (e / rk).asin()).to_degrees())
}

/// The body outline of every unit at every step, as convex quadrilaterals
/// (front left, front right, rear right, rear left).
pub fn outlines(v: &Vehicle, s: &Swept) -> Vec<[P; 4]> {
    s.poses
        .iter()
        .flat_map(|ps| ps.iter().zip(&v.units).map(|(p, u)| [p.at(0.0, u.width / 2.0), p.at(0.0, -u.width / 2.0), p.at(u.length, -u.width / 2.0), p.at(u.length, u.width / 2.0)]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_run_keeps_the_vehicle_on_the_line() {
        let v = crate::library::vehicles().iter().find(|v| v.units.len() > 1).unwrap();
        let s = drive(v, &[[0.0, 0.0], [60.0, 0.0]], 0.1, 20.0).unwrap();
        for trace in &s.body {
            for p in trace {
                assert!(p[1].abs() < v.width() / 2.0 + 1e-6, "{p:?}");
            }
        }
        assert!(s.max_steer_deg < 1e-6);
        assert!(s.ghosts.len() >= 3);
    }

    #[test]
    fn a_quarter_turn_cuts_the_corner() {
        let v = crate::library::vehicles().iter().find(|v| v.category == "car").unwrap();
        // 10 m straight, a quarter circle of 8 m, 10 m straight up.
        let mut path = vec![[0.0, 0.0], [10.0, 0.0]];
        for i in 1..=30 {
            let a = -std::f64::consts::FRAC_PI_2 + i as f64 / 30.0 * std::f64::consts::FRAC_PI_2;
            path.push([10.0 + 8.0 * a.cos(), 8.0 + 8.0 * a.sin()]);
        }
        path.push([18.0, 18.0]);
        let s = drive(v, &path, 0.05, 0.0).unwrap();
        // The rear axle cuts inside the front axle's circle (off-tracking).
        let inner_rear = s.wheels.iter().flat_map(|w| w.iter()).map(|p| (p[0] - 10.0).hypot(p[1] - 8.0)).fold(f64::MAX, f64::min);
        assert!(inner_rear < 8.0 - 0.5, "inner wheel radius {inner_rear}");
        assert!(s.max_steer_deg > 10.0 && s.max_steer_deg < 30.0, "{}", s.max_steer_deg);
    }

    fn vehicle(id: &str) -> &'static Vehicle {
        crate::library::find(id).unwrap_or_else(|| panic!("{id} not in the library"))
    }

    /// A circle of radius `r` around the origin, from angle 0 counter-clockwise,
    /// after `lead` metres of straight run-in along the tangent.
    fn circle_path(r: f64, sweep_deg: f64, lead: f64) -> Vec<P> {
        let mut p = vec![[r, -lead]];
        let n = (sweep_deg / 1.0).ceil() as usize;
        for k in 0..=n {
            let a = (k as f64 / n as f64 * sweep_deg).to_radians();
            p.push([r * a.cos(), r * a.sin()]);
        }
        p
    }

    #[test]
    fn reversing_a_car_puts_its_rear_axle_on_the_path() {
        let v = vehicle("car_vw_golf");
        let path = circle_path(10.0, 90.0, 5.0);
        let s = drive_reverse(v, &path, 0.05, 0.0).unwrap();
        let (_, r) = axles(&v.units[0]).unwrap();
        for ps in &s.poses {
            // The rear axle centre lies on the path (radius 10 on the arc).
            let a = ps[0].at(r, 0.0);
            if a[1] > 0.5 {
                assert!((a[0].hypot(a[1]) - 10.0).abs() < 0.01, "{a:?}");
            }
        }
        // Reversing: the front points against the direction of travel, so at
        // the end of the quarter turn (travelling −x) it points +x.
        let end = s.poses.last().unwrap()[0];
        assert!(end.dir[0] > 0.99, "{:?}", end.dir);
        // tan δ = wheelbase / 10 m.
        let (f, r) = axles(&v.units[0]).unwrap();
        let want = ((r - f) / 10.0).atan().to_degrees();
        assert!((s.max_steer_deg - want).abs() < 1.0, "{} vs {want}", s.max_steer_deg);
        assert!(s.reverse);
    }

    #[test]
    fn a_semitrailer_reversing_round_a_circle_settles_at_the_no_slip_angle() {
        // The trailer's axle group follows a circle of 20 m backwards. In the
        // steady state the trailer's heading is tilted outward from the
        // kingpin circle by asin(wb_t / R_k), the tractor's by asin(e / R_k):
        // the articulation is their difference.
        let v = vehicle("eu_tractor_semitrailer_16_5");
        let rt = 20.0;
        let s = drive_reverse(v, &circle_path(rt, 200.0, 20.0), 0.05, 0.0).unwrap();
        let (_, r0) = axles(&v.units[0]).unwrap();
        let (_, r1) = axles(&v.units[1]).unwrap();
        let e = r0 - crate::library::draw::hitch_x(&v.units[0]);
        let wb_t = r1 - crate::library::draw::kingpin_x(&v.units[1]);
        let rk = (rt * rt + wb_t * wb_t).sqrt();
        let want = ((wb_t / rk).asin() - (e / rk).asin()).to_degrees();
        // Halfway round the circle the tractor has settled.
        let mid = &s.poses[s.poses.len() / 2];
        let got = articulation(&mid[0], &mid[1]);
        assert!((got - want).abs() < 0.5, "articulation {got:.2}° vs {want:.2}°");
        // The trailer axle stays on the path, the tractor follows round.
        let ta = mid[1].at(r1, 0.0);
        assert!((ta[0].hypot(ta[1]) - rt).abs() < 0.02, "{ta:?}");
        // The coupling stays together: tractor hitch = trailer kingpin.
        for ps in s.poses.iter().step_by(50) {
            let h = ps[0].at(crate::library::draw::hitch_x(&v.units[0]), 0.0);
            let k = ps[1].at(crate::library::draw::kingpin_x(&v.units[1]), 0.0);
            assert!(len(sub(h, k)) < 1e-6, "{h:?} {k:?}");
        }
        assert!(s.max_articulation_deg >= got - 1e-9);
    }

    #[test]
    fn a_tight_reverse_turn_jackknifes() {
        // The jackknife angle of the EU semitrailer from its 12.5 m circle:
        // the tractor's rear axle runs on about 10 m at full lock.
        let v = vehicle("eu_tractor_semitrailer_16_5");
        let limit = jackknife_angle(v).unwrap();
        assert!(limit > 30.0 && limit < 60.0, "{limit}");
        // Round a 4 m circle backwards the trailer needs more than that.
        let s = drive_reverse(v, &circle_path(4.0, 120.0, 15.0), 0.05, 0.0).unwrap();
        assert!(s.max_articulation_deg > limit, "{} vs {limit}", s.max_articulation_deg);
        // Gently, the angle stays small.
        let s = drive_reverse(v, &circle_path(60.0, 30.0, 15.0), 0.05, 0.0).unwrap();
        assert!(s.max_articulation_deg < 15.0, "{}", s.max_articulation_deg);
    }

    #[test]
    fn a_drawbar_trailer_reverses_with_its_truck_following() {
        // A truck with a drawbar trailer: the truck's hitch is behind its
        // rear axle (e < 0), integrated in the reversing direction.
        let v = vehicle("eu_truck_trailer_18_75");
        let s = drive_reverse(v, &[[0.0, 0.0], [-40.0, 0.0]], 0.05, 10.0).unwrap();
        for ps in &s.poses {
            assert!(articulation(&ps[0], &ps[1]) < 1e-6);
            assert!(ps[0].dir[0] > 0.999, "front points +x while reversing −x");
        }
        assert!(s.ghosts.len() >= 4);
        // Couplings hold on a curve too.
        let s = drive_reverse(v, &circle_path(25.0, 60.0, 10.0), 0.05, 0.0).unwrap();
        for ps in s.poses.iter().step_by(40) {
            let h = ps[0].at(crate::library::draw::hitch_x(&v.units[0]), 0.0);
            let k = ps[1].at(crate::library::draw::kingpin_x(&v.units[1]), 0.0);
            assert!(len(sub(h, k)) < 1e-6);
        }
    }

    #[test]
    fn forward_articulation_is_reported() {
        let v = vehicle("eu_tractor_semitrailer_16_5");
        let s = drive(v, &circle_path(12.5, 180.0, 10.0), 0.05, 0.0).unwrap();
        assert!(s.max_articulation_deg > 10.0 && s.max_articulation_deg < 70.0, "{}", s.max_articulation_deg);
    }
}
