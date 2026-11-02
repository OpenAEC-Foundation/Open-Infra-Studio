//! Pure, host-independent geometry shared by preview, placement and regeneration.
use super::{Item, Placement};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Primitive {
    Hatch {
        boundary: Vec<[f64; 3]>,
        strokes: Vec<Vec<[f64; 3]>>,
        patterns: Vec<[f64; 2]>,
        origin: [f64; 3],
    },
    Poly {
        points: Vec<[f64; 3]>,
        closed: bool,
    },
    Circle {
        center: [f64; 3],
        radius: f64,
    },
    Text {
        at: [f64; 3],
        value: String,
        height: f64,
        rotation: f64,
    },
}
#[derive(Clone, Debug, Serialize)]
pub struct Part {
    pub object_id: u32,
    pub element: String,
    pub geometry: Primitive,
    pub primary: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Port {
    pub id: String,
    pub at: [f64; 3],
    pub direction: [f64; 2],
    pub system: String,
    pub diameter: f64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Built {
    pub parts: Vec<Part>,
    pub ports: Vec<Port>,
    pub quantity: f64,
    pub unit: String,
    pub warnings: Vec<String>,
    pub elevation_known: bool,
    pub boundary: Vec<[f64; 3]>,
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}
fn cross(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}
pub fn path_length(points: &[[f64; 2]]) -> f64 {
    points.windows(2).map(|w| dist(w[0], w[1])).sum()
}
/// Schematic material pattern clipped to an arbitrary simple polygon.
fn pattern(polygon: &[[f64; 2]], spacing: f64, angle: f64) -> Vec<Vec<[f64; 2]>> {
    let (s, c) = angle.sin_cos();
    let pts: Vec<_> = polygon
        .iter()
        .map(|p| [c * p[0] + s * p[1], -s * p[0] + c * p[1]])
        .collect();
    let min = pts.iter().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let max = pts.iter().map(|p| p[1]).fold(f64::NEG_INFINITY, f64::max);
    let spacing = spacing.max((max - min) / 180.).max(0.01);
    let mut out = vec![];
    let mut y = (min / spacing).floor() * spacing + spacing;
    while y < max {
        let mut cuts = vec![];
        for k in 0..pts.len() {
            let a = pts[k];
            let b = pts[(k + 1) % pts.len()];
            if (a[1] <= y && b[1] > y) || (b[1] <= y && a[1] > y) {
                cuts.push(a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]));
            }
        }
        cuts.sort_by(f64::total_cmp);
        for pair in cuts.chunks_exact(2) {
            out.push(
                pair.iter()
                    .map(|x| [c * x - s * y, s * x + c * y])
                    .collect(),
            );
        }
        y += spacing;
    }
    out
}
fn on_segment(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> bool {
    cross(a, b, p).abs() < 1e-10
        && p[0] >= a[0].min(b[0]) - 1e-10
        && p[0] <= a[0].max(b[0]) + 1e-10
        && p[1] >= a[1].min(b[1]) - 1e-10
        && p[1] <= a[1].max(b[1]) + 1e-10
}
fn intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> bool {
    let (u, v, w, x) = (
        cross(a, b, c),
        cross(a, b, d),
        cross(c, d, a),
        cross(c, d, b),
    );
    (u * v < 0. && w * x < 0.)
        || on_segment(a, b, c)
        || on_segment(a, b, d)
        || on_segment(c, d, a)
        || on_segment(c, d, b)
}
pub fn polygon_area(points: &[[f64; 2]]) -> Result<f64, String> {
    let mut pts = points.to_vec();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    let n = pts.len();
    if !(3..=4096).contains(&n) {
        return Err("vlak: 3 t/m 4096 hoekpunten vereist".into());
    }
    for i in 0..n {
        if dist(pts[i], pts[(i + 1) % n]) < 1e-8 {
            return Err("vlak bevat dubbele hoekpunten".into());
        }
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            if intersect(pts[i], pts[(i + 1) % n], pts[j], pts[(j + 1) % n]) {
                return Err("vlak snijdt zichzelf".into());
            }
        }
    }
    let area = (0..n)
        .map(|i| pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1])
        .sum::<f64>()
        .abs()
        / 2.;
    if area < 1e-8 {
        Err("vlak heeft geen oppervlakte".into())
    } else {
        Ok(area)
    }
}
/// Mitered side of a polyline, with bounded joins near a reversal.
pub fn offset(points: &[[f64; 2]], distance: f64) -> Vec<[f64; 2]> {
    let normals: Vec<_> = points
        .windows(2)
        .map(|w| {
            let d = dist(w[0], w[1]);
            [-(w[1][1] - w[0][1]) / d, (w[1][0] - w[0][0]) / d]
        })
        .collect();
    points
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let a = normals[i.saturating_sub(1)];
            let b = normals[i.min(normals.len() - 1)];
            let n = [a[0] + b[0], a[1] + b[1]];
            let d = (n[0] * b[0] + n[1] * b[1]).max(0.25);
            [p[0] + distance * n[0] / d, p[1] + distance * n[1] / d]
        })
        .collect()
}

pub fn build(item: &Item, p: &Placement) -> Result<Built, String> {
    item.validate(p)?;
    let (l, w) = (item.value(p, "length"), item.value(p, "width"));
    let diameter = if item.system == "curb" {
        w * 1000.
    } else {
        item.value(p, "diameter")
    };
    let slope = item.value(p, "slope");
    let mut out = Built {
        parts: vec![],
        ports: vec![],
        quantity: 1.,
        unit: item.quantity_unit.clone(),
        warnings: item.conflicts.clone(),
        elevation_known: p.elevation_known,
        boundary: vec![],
    };
    if !p.elevation_known {
        out.warnings.push(
            "Hoogte niet vastgesteld: Z is schematisch, geen vastgesteld NAP- of BOB-peil.".into(),
        );
    }
    let mut local: Vec<(u32, String, Primitive)> = vec![];
    let mut ports: Vec<Port> = vec![];
    let elem = item.element.clone();
    let mut poly = |pts: Vec<[f64; 2]>, closed: bool| {
        local.push((
            item.object_id,
            elem.clone(),
            Primitive::Poly {
                points: pts.into_iter().map(|v| [v[0], v[1], 0.]).collect(),
                closed,
            },
        ))
    };
    let rect = |x: f64, y: f64, len: f64, width: f64| {
        vec![[x, y], [x + len, y], [x + len, y + width], [x, y + width]]
    };
    let mut path = if p.path.is_empty() {
        vec![[0., 0.], [l, 0.]]
    } else {
        p.path.clone()
    };
    if item.kind == "area" && p.path.is_empty() {
        path = rect(0., 0., l, w)
    }
    if path.len() > 4096 {
        return Err("maximaal 4096 padpunten".into());
    }
    if item.kind == "line" {
        if path.len() < 2 || path.windows(2).any(|v| dist(v[0], v[1]) < 1e-8) {
            return Err("lijn: verschillende opeenvolgende punten vereist".into());
        }
        out.quantity = path_length(&path);
        poly(path.clone(), false);
        if matches!(item.shape.as_str(), "curb" | "pipe" | "fence") {
            let width = if diameter > 0. { diameter / 1000. } else { w };
            poly(offset(&path, width / 2.), false);
            poly(offset(&path, -width / 2.), false);
        }
        if item.port_count > 0 {
            let a = path[0];
            let b = path[1];
            let z = path[path.len() - 1];
            let y = path[path.len() - 2];
            let d = dist(a, b);
            let e = dist(y, z);
            ports.push(Port {
                id: "in".into(),
                at: [a[0], a[1], 0.],
                direction: [(a[0] - b[0]) / d, (a[1] - b[1]) / d],
                system: item.system.clone(),
                diameter,
            });
            ports.push(Port {
                id: "out".into(),
                at: [z[0], z[1], -slope * out.quantity],
                direction: [(z[0] - y[0]) / e, (z[1] - y[1]) / e],
                system: item.system.clone(),
                diameter,
            });
        }
        // Centre line carries actual invert elevations, decorative edges follow it.
        for (_, _, g) in &mut local {
            if let Primitive::Poly { points, .. } = g {
                let mut station = 0.;
                for (k, point) in points.iter_mut().enumerate() {
                    if k > 0 {
                        station += dist(path[k - 1], path[k]);
                    }
                    point[2] = -slope * station;
                }
            }
        }
    } else if item.kind == "area" {
        out.quantity = polygon_area(&path)?;
        if path.first() == path.last() {
            path.pop();
        }
        out.boundary = path.iter().map(|p| [p[0], p[1], 0.]).collect();
        let patterns: Vec<(f64,f64)> = if item.material.contains("baksteen") {
            vec![
                (0.15, std::f64::consts::FRAC_PI_4),
                (0.3, -std::f64::consts::FRAC_PI_4),
            ]
        } else if item.material == "grasbeton" {
            vec![(0.4, 0.), (0.6, std::f64::consts::FRAC_PI_2)]
        } else if item.id == "betontegel-300" {
            vec![(0.3, 0.), (0.3, std::f64::consts::FRAC_PI_2)]
        } else if item.material.contains("asfalt") {
            vec![(0.4, std::f64::consts::FRAC_PI_4)]
        } else {
            vec![(0.6, 0.)]
        };
        poly(path.clone(), true);
        let patterns: Vec<_> = patterns
            .into_iter()
            .map(|(spacing, angle)| {
                let (s, c) = angle.sin_cos();
                let mut min = f64::INFINITY;
                let mut max = f64::NEG_INFINITY;
                for v in &path {
                    let y = -s * v[0] + c * v[1];
                    min = min.min(y);
                    max = max.max(y);
                }
                (spacing.max((max - min) / 180.).max(0.01), angle)
            })
            .collect();
        let strokes = patterns
            .iter()
            .flat_map(|(spacing, angle)| pattern(&path, *spacing, *angle))
            .map(|line| line.into_iter().map(|v| [v[0], v[1], 0.]).collect())
            .collect();
        let element = if crate::nlcs::object(item.object_id)
            .is_some_and(|o| crate::nlcs::element_allowed(o, "A"))
        {
            "A"
        } else {
            "G"
        };
        local.push((
            item.object_id,
            element.into(),
            Primitive::Hatch {
                boundary: path.iter().map(|v| [v[0], v[1], 0.]).collect(),
                strokes,
                patterns: patterns.into_iter().map(|(s, a)| [s, a]).collect(),
                origin: [0.; 3],
            },
        ));
    } else if item.kind == "profile" {
        let mut depth = 0.;
        for (k, component) in item.components.iter().enumerate() {
            let thickness = item.value(p, &format!("layer_{}", k + 1));
            let obj = crate::nlcs::object(component.object_id).ok_or("onbekend componentobject")?;
            if !crate::nlcs::element_allowed(obj, "G") {
                return Err("profielcomponent heeft geen geometrie".into());
            }
            let pts = rect(0., -depth - thickness, l, thickness)
                .into_iter()
                .map(|a| [a[0], a[1], 0.])
                .collect();
            local.push((
                component.object_id,
                "G".into(),
                Primitive::Poly {
                    points: pts,
                    closed: true,
                },
            ));
            local.push((
                component.object_id,
                "T".into(),
                Primitive::Text {
                    at: [l + 0.15, -depth - thickness / 2., 0.],
                    value: format!("{} {:.0} mm", component.material, thickness * 1000.),
                    height: 0.0025 * p.scale,
                    rotation: 0.,
                },
            ));
            depth += thickness;
        }
        out.warnings.push(
            "Schematisch profiel; niveaus, laagdikten en engineering projectspecifiek controleren."
                .into(),
        );
    } else if item.kind == "annotation" {
        let value = match item.id.as_str() {
            "afschotpijl" => format!("{:.1} ‰ →", slope * 1000.),
            "profielmarkering" => "A — A".into(),
            _ => {
                if p.elevation_known {
                    format!("{:+.3} NAP", p.at[2])
                } else {
                    "NAP ?".into()
                }
            }
        };
        local.push((
            item.object_id,
            "T".into(),
            Primitive::Text {
                at: [0.; 3],
                value,
                height: 0.0025 * p.scale,
                rotation: 0.,
            },
        ));
    } else {
        match item.shape.as_str() {
            "tree" => {
                poly(vec![[-w / 8., 0.], [w / 8., 0.]], false);
                local.push((
                    item.object_id,
                    elem.clone(),
                    Primitive::Circle {
                        center: [0.; 3],
                        radius: w / 2.,
                    },
                ));
            }
            "lamp" | "hydrant" => {
                poly(vec![[-w, 0.], [w, 0.]], false);
                poly(vec![[0., -w], [0., w]], false);
                local.push((
                    item.object_id,
                    elem.clone(),
                    Primitive::Circle {
                        center: [0.; 3],
                        radius: w / 2.,
                    },
                ));
            }
            "valve" | "check" => {
                poly(
                    vec![
                        [0., -w / 2.],
                        [l, w / 2.],
                        [l, -w / 2.],
                        [0., w / 2.],
                        [0., -w / 2.],
                    ],
                    false,
                );
            }
            "bend" => {
                let a = item.value(p, "angle").to_radians();
                let pts: Vec<_> = (0..=16)
                    .map(|k| {
                        let t = a * k as f64 / 16.;
                        [l * t.sin(), l * (1. - t.cos())]
                    })
                    .collect();
                poly(pts, false);
            }
            "tee" => {
                let a = item.value(p, "angle").to_radians();
                poly(vec![[0., 0.], [l, 0.]], false);
                poly(
                    vec![[l / 2., 0.], [l / 2. + w * a.cos(), w * a.sin()]],
                    false,
                );
            }
            "reducer" => {
                let d = item.value(p, "out_diameter") / 1000.;
                poly(
                    vec![
                        [0., -diameter / 2000.],
                        [l, -d / 2.],
                        [l, d / 2.],
                        [0., diameter / 2000.],
                    ],
                    true,
                );
            }
            "vent" => {
                poly(vec![[0., 0.], [0., w], [w / 2., w]], false);
            }
            "bench" => {
                poly(rect(0., -w / 2., l, w), true);
                poly(vec![[0.15, -w / 2.], [0.15, w / 2.]], false);
                poly(vec![[l - 0.15, -w / 2.], [l - 0.15, w / 2.]], false);
            }
            "rack" => {
                poly(vec![[0., 0.], [l, 0.]], false);
                poly(vec![[0., -w], [0., w]], false);
                poly(vec![[l, -w], [l, w]], false);
            }
            "sign" => {
                poly(rect(-l / 2., -l / 2., l, l), true);
                poly(vec![[0., -l / 2.], [0., -l]], false);
            }
            "gate" => {
                poly(vec![[0., 0.], [l, 0.], [l * 0.707, l * 0.707]], false);
            }
            "posts" => {
                for x in [0., l / 2., l] {
                    local.push((
                        item.object_id,
                        elem.clone(),
                        Primitive::Circle {
                            center: [x, 0., 0.],
                            radius: w / 12.,
                        },
                    ));
                }
            }
            "log" => {
                poly(rect(0., -w / 2., l, w), true);
            }
            _ => {
                poly(rect(0., -w / 2., l, w), true);
                if item.shape == "chamber" {
                    poly(vec![[0., -w / 2.], [l, w / 2.]], false);
                    poly(vec![[0., w / 2.], [l, -w / 2.]], false);
                }
            }
        }
        if item.port_count > 0 {
            let specs: Vec<(&str, [f64; 3], [f64; 2])> = if item.shape == "chamber" {
                vec![
                    ("in", [0., 0., 0.], [-1., 0.]),
                    ("out", [l, 0., 0.], [1., 0.]),
                    ("left", [l / 2., w / 2., 0.], [0., 1.]),
                    ("right", [l / 2., -w / 2., 0.], [0., -1.]),
                ]
            } else if item.shape == "bend" {
                let a = item.value(p, "angle").to_radians();
                vec![
                    ("in", [0.; 3], [-1., 0.]),
                    (
                        "out",
                        [l * a.sin(), l * (1. - a.cos()), 0.],
                        [a.cos(), a.sin()],
                    ),
                ]
            } else if item.shape == "tee" {
                let a = item.value(p, "angle").to_radians();
                vec![
                    ("in", [0.; 3], [-1., 0.]),
                    ("out", [l, 0., 0.], [1., 0.]),
                    (
                        "branch",
                        [l / 2. + w * a.cos(), w * a.sin(), 0.],
                        [a.cos(), a.sin()],
                    ),
                ]
            } else {
                vec![
                    ("in", [0.; 3], [-1., 0.]),
                    ("out", [l, 0., 0.], [1., 0.]),
                    ("branch", [l / 2., w, 0.], [0., 1.]),
                    ("aux", [l / 2., -w, 0.], [0., -1.]),
                ]
            };
            for (k, (id, at, direction)) in specs.into_iter().take(item.port_count).enumerate() {
                ports.push(Port {
                    id: id.into(),
                    at,
                    direction,
                    system: item.system.clone(),
                    diameter: if item.shape == "reducer" && k == 1 {
                        item.value(p, "out_diameter")
                    } else {
                        diameter
                    },
                });
            }
        }
    }
    let (s, c) = p.rotation.sin_cos();
    let transform = |a: [f64; 3]| {
        [
            p.at[0] + c * a[0] - s * a[1],
            p.at[1] + s * a[0] + c * a[1],
            p.at[2] + a[2],
        ]
    };
    for (index, (obj, element, g)) in local.into_iter().enumerate() {
        let geometry = match g {
            Primitive::Hatch {
                boundary,
                strokes,
                patterns,
                origin,
            } => Primitive::Hatch {
                boundary: boundary.into_iter().map(transform).collect(),
                strokes: strokes
                    .into_iter()
                    .map(|line| line.into_iter().map(transform).collect())
                    .collect(),
                patterns: patterns
                    .into_iter()
                    .map(|v| [v[0], v[1] + p.rotation])
                    .collect(),
                origin: transform(origin),
            },
            Primitive::Poly { points, closed } => Primitive::Poly {
                points: points.into_iter().map(transform).collect(),
                closed,
            },
            Primitive::Circle { center, radius } => Primitive::Circle {
                center: transform(center),
                radius,
            },
            Primitive::Text {
                at,
                value,
                height,
                rotation,
            } => Primitive::Text {
                at: transform(at),
                value,
                height,
                rotation: rotation + p.rotation,
            },
        };
        out.parts.push(Part {
            object_id: obj,
            element,
            geometry,
            primary: index == 0,
        });
    }
    for mut port in ports {
        port.at = transform(port.at);
        let d = port.direction;
        port.direction = [c * d[0] - s * d[1], s * d[0] + c * d[1]];
        out.ports.push(port);
    }
    out.boundary = out.boundary.into_iter().map(transform).collect();
    if out.parts.is_empty() {
        return Err("geen geldige geometrie".into());
    }
    Ok(out)
}
