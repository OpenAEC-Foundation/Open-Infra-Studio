//! Quantities per NLCS layer: count, length
//! and area of the elements, grouped by layer, with the NLCS object, status
//! and element read from the layer name. Lengths in m, areas in m².

use std::collections::{BTreeMap, HashSet};
use std::f64::consts::PI;

use ocs_plugin_api::host::codec::entities::{BoundaryEdge, Hatch};
use ocs_plugin_api::host::codec::{CadDocument, EntityType, Handle};

use crate::nlcs;

#[derive(Clone, Debug, Default)]
pub struct Row {
    /// Explicit measure of a parametric library object; decoration is excluded.
    pub quantity_unit: String,
    pub layer: String,
    /// From the layer name when it is an NLCS name.
    pub status: String,
    pub discipline: String,
    pub group: String,
    pub object: String,
    pub element: String,
    pub count: usize,
    /// Open lines and the outline of closed figures, in m.
    pub length: f64,
    /// Closed polylines, circles, ellipses and hatches, in m².
    pub area: f64,
    /// Closed figures among `count`.
    pub closed: usize,
    /// Copies of an element already counted on this layer (same shape, either
    /// direction, and the same object data): not in `count`, `length` and `area`.
    pub duplicates: usize,
    /// Their length, in m.
    pub duplicate_length: f64,
}

impl Row {
    pub fn nlcs(&self) -> bool {
        !self.group.is_empty()
    }
    /// Sheet and annotation layers (AL, ZZ): frames, legends, profile
    /// bands. They are no quantities of the design.
    pub fn annotation(&self) -> bool {
        self.discipline == "AL" || self.discipline == "ZZ"
    }
    /// The object for people: "Geslotenverharding asfalt".
    pub fn object_label(&self) -> String {
        if self.nlcs() { nlcs::humanize(&self.object) } else { self.layer.clone() }
    }
    /// The quantity that matters for this layer: area for hatches and
    /// mostly-closed layers, otherwise length, or the count.
    pub fn main(&self) -> (f64, &'static str) {
        match self.quantity_unit.as_str() {
            "m" => return (self.length, "m"),
            "m2" => return (self.area, "m²"),
            "st" => return (self.count as f64, "st"),
            _ => {}
        }
        if self.element == "A" || (self.closed * 2 > self.count && self.area > 0.0) {
            (self.area, "m²")
        } else if self.length > 0.0 && self.element != "S" && self.element != "T" {
            (self.length, "m")
        } else {
            (self.count as f64, "st")
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Quantities {
    pub rows: Vec<Row>,
    pub entities: usize,
    pub from_selection: bool,
    /// Double-drawn elements: (layer, the element counted, its copy).
    pub duplicates: Vec<(String, Handle, Handle)>,
}

impl Quantities {
    /// "3 dubbel getekende elementen (12,5 m) niet meegeteld, op N-WE-…": for messages.
    pub fn duplicates_note(&self) -> Option<String> {
        if self.duplicates.is_empty() {
            return None;
        }
        let len: f64 = self.rows.iter().map(|r| r.duplicate_length).sum();
        let mut layers: Vec<&str> = self.rows.iter().filter(|r| r.duplicates > 0).map(|r| r.layer.as_str()).collect();
        let more = layers.len().saturating_sub(3);
        layers.truncate(3);
        Some(format!(
            "{} {} ({} m) {} {}{}",
            self.duplicates.len(),
            crate::lang::tr("dubbel getekende elementen", "double-drawn elements"),
            format!("{len:.1}").replace('.', ","),
            crate::lang::tr("niet meegeteld, op", "not counted, on"),
            layers.join(", "),
            if more > 0 { format!(" (+{more})") } else { String::new() }
        ))
    }
}

/// The shape of a line-like element in whole millimetres, the same for a
/// copy drawn the other way round or (closed) from another vertex, with a tag
/// per kind. `None` for elements that are not compared (texts, hatches, …).
fn shape_key(e: &EntityType, f: f64) -> Option<Vec<i64>> {
    let mm = |v: f64| (v / f * 1000.0).round() as i64;
    let b = |v: f64| (v * 1e4).round() as i64;
    // Points as [x, y, z, bulge-to-next], then canonical for direction and start.
    let poly = |tag: i64, pts: Vec<[i64; 4]>, closed: bool| -> Vec<i64> {
        let n = pts.len();
        let reversed: Vec<[i64; 4]> = (0..n)
            .rev()
            .map(|i| {
                let bulge = if closed { pts[(i + n - 1) % n][3] } else if i > 0 { pts[i - 1][3] } else { 0 };
                [pts[i][0], pts[i][1], pts[i][2], -bulge]
            })
            .collect();
        let mut options = vec![pts.clone(), reversed];
        if closed && n > 0 {
            options = options
                .into_iter()
                .map(|v| {
                    let start = (0..n).min_by_key(|&i| (v[i][0], v[i][1], v[i][2])).unwrap_or(0);
                    v[start..].iter().chain(v[..start].iter()).copied().collect()
                })
                .collect();
        }
        let best = options.into_iter().min().unwrap_or_default();
        let mut key = vec![tag, closed as i64];
        key.extend(best.iter().flatten());
        key
    };
    match e {
        EntityType::Line(l) => Some(poly(1, vec![[mm(l.start.x), mm(l.start.y), mm(l.start.z), 0], [mm(l.end.x), mm(l.end.y), mm(l.end.z), 0]], false)),
        // A line is a two-point polyline: an open polyline copy of a line is a duplicate too.
        EntityType::LwPolyline(p) if p.vertices.len() >= 2 => {
            let tag = if p.vertices.len() == 2 && !p.is_closed && p.vertices[0].bulge == 0.0 { 1 } else { 2 };
            Some(poly(tag, p.vertices.iter().map(|v| [mm(v.location.x), mm(v.location.y), mm(p.elevation), b(v.bulge)]).collect(), p.is_closed))
        }
        EntityType::Polyline2D(p) if p.vertices.len() >= 2 => {
            Some(poly(2, p.vertices.iter().map(|v| [mm(v.location.x), mm(v.location.y), mm(v.location.z), b(v.bulge)]).collect(), p.flags.is_closed()))
        }
        EntityType::Polyline3D(p) if p.vertices.len() >= 2 => {
            Some(poly(3, p.vertices.iter().map(|v| [mm(v.position.x), mm(v.position.y), mm(v.position.z), 0]).collect(), p.flags.closed))
        }
        // Hoekafronding geeft bij grote stralen geen millimetertolerantie.
        // Bogen blijven meetellen totdat hun echte geometrie wordt vergeleken.
        EntityType::Arc(_) => None,
        EntityType::Circle(c) => Some(vec![5, mm(c.center.x), mm(c.center.y), mm(c.center.z), mm(c.radius)]),
        _ => None,
    }
}

/// Measure one element in drawing units: (length, area, closed).
pub fn measure(e: &EntityType) -> (f64, f64, bool) {
    match e {
        EntityType::Line(l) => ((l.end.x - l.start.x).hypot(l.end.y - l.start.y).hypot(l.end.z - l.start.z), 0.0, false),
        EntityType::LwPolyline(p) => {
            let pts: Vec<([f64; 2], f64)> = p.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge)).collect();
            ring_measure(&pts, p.is_closed)
        }
        EntityType::Polyline2D(p) => {
            let pts: Vec<([f64; 2], f64)> = p.vertices.iter().map(|v| ([v.location.x, v.location.y], v.bulge)).collect();
            ring_measure(&pts, p.flags.is_closed())
        }
        EntityType::Polyline3D(p) => {
            let v: Vec<[f64; 3]> = p.vertices.iter().map(|v| [v.position.x, v.position.y, v.position.z]).collect();
            let closed = p.flags.closed;
            let mut len: f64 = v.windows(2).map(|w| dist3(w[0], w[1])).sum();
            if closed && v.len() > 2 {
                len += dist3(v[v.len() - 1], v[0]);
            }
            let flat: Vec<([f64; 2], f64)> = v.iter().map(|p| ([p[0], p[1]], 0.0)).collect();
            let area = if closed { ring_measure(&flat, true).1 } else { 0.0 };
            (len, area, closed)
        }
        EntityType::Arc(a) => {
            let mut sweep = a.end_angle - a.start_angle;
            if sweep <= 0.0 {
                sweep += 2.0 * PI;
            }
            (a.radius * sweep, 0.0, false)
        }
        EntityType::Circle(c) => (2.0 * PI * c.radius, PI * c.radius * c.radius, true),
        EntityType::Ellipse(el) => {
            let a = el.major_axis.x.hypot(el.major_axis.y).hypot(el.major_axis.z);
            let b = a * el.minor_axis_ratio;
            let full = (el.end_parameter - el.start_parameter - 2.0 * PI).abs() < 1e-6 || (el.end_parameter - el.start_parameter).abs() < 1e-9;
            // Ramanujan's perimeter, scaled for a partial ellipse.
            let h = ((a - b) / (a + b)).powi(2);
            let perimeter = PI * (a + b) * (1.0 + 3.0 * h / (10.0 + (4.0 - 3.0 * h).sqrt()));
            if full {
                (perimeter, PI * a * b, true)
            } else {
                let mut sweep = el.end_parameter - el.start_parameter;
                if sweep <= 0.0 {
                    sweep += 2.0 * PI;
                }
                (perimeter * sweep / (2.0 * PI), 0.0, false)
            }
        }
        EntityType::Spline(s) => {
            let pts: Vec<[f64; 3]> = if s.fit_points.len() >= 2 { s.fit_points.iter().map(|p| [p.x, p.y, p.z]).collect() } else { s.control_points.iter().map(|p| [p.x, p.y, p.z]).collect() };
            (pts.windows(2).map(|w| dist3(w[0], w[1])).sum(), 0.0, false)
        }
        EntityType::Hatch(h) => (0.0, hatch_area(h), true),
        EntityType::Solid(s) => {
            let c = [s.first_corner, s.second_corner, s.fourth_corner, s.third_corner];
            let pts: Vec<([f64; 2], f64)> = c.iter().map(|p| ([p.x, p.y], 0.0)).collect();
            (0.0, ring_measure(&pts, true).1, true)
        }
        _ => (0.0, 0.0, false),
    }
}

fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
}

/// Length and area of a polyline with bulges (arc segments). The area of a
/// closed one is the polygon area plus the arc segments, as CAD programs report
/// it.
fn ring_measure(pts: &[([f64; 2], f64)], closed: bool) -> (f64, f64, bool) {
    let n = pts.len();
    if n < 2 {
        return (0.0, 0.0, closed);
    }
    let segments = if closed { n } else { n - 1 };
    let (mut len, mut twice_area) = (0.0, 0.0);
    for i in 0..segments {
        let (a, bulge) = pts[i];
        let b = pts[(i + 1) % n].0;
        let chord = (b[0] - a[0]).hypot(b[1] - a[1]);
        twice_area += a[0] * b[1] - b[0] * a[1];
        if bulge.abs() < 1e-12 || chord == 0.0 {
            len += chord;
        } else {
            let theta = 4.0 * bulge.atan();
            let radius = chord / (2.0 * (theta / 2.0).sin()).abs();
            len += radius * theta.abs();
            // Circular segment between chord and arc, signed by the bulge.
            let segment = radius * radius * (theta.abs() - theta.abs().sin()) / 2.0;
            twice_area += 2.0 * segment * bulge.signum();
        }
    }
    let area = if closed && n > 2 { (twice_area / 2.0).abs() } else { 0.0 };
    (len, area, closed)
}

/// Area of a hatch: the outer boundaries minus the islands in them.
fn hatch_area(h: &Hatch) -> f64 {
    let mut areas = Vec::new();
    for path in &h.paths {
        let mut pts: Vec<([f64; 2], f64)> = Vec::new();
        for edge in &path.edges {
            match edge {
                BoundaryEdge::Polyline(p) => {
                    // The bulge is carried in z.
                    pts.extend(p.vertices.iter().map(|v| ([v.x, v.y], v.z)));
                }
                BoundaryEdge::Line(l) => {
                    if pts.last().is_none_or(|q| (q.0[0] - l.start.x).abs() > 1e-9 || (q.0[1] - l.start.y).abs() > 1e-9) {
                        pts.push(([l.start.x, l.start.y], 0.0));
                    }
                    pts.push(([l.end.x, l.end.y], 0.0));
                }
                BoundaryEdge::CircularArc(a) => {
                    let mut sweep = a.end_angle - a.start_angle;
                    if sweep <= 0.0 {
                        sweep += 2.0 * PI;
                    }
                    let steps = ((sweep / (PI / 36.0)).ceil() as usize).max(2);
                    for k in 0..=steps {
                        let t = if a.counter_clockwise { a.start_angle + sweep * k as f64 / steps as f64 } else { -(a.start_angle + sweep * k as f64 / steps as f64) };
                        pts.push(([a.center.x + a.radius * t.cos(), a.center.y + a.radius * t.sin()], 0.0));
                    }
                }
                BoundaryEdge::EllipticArc(e) => {
                    let (mx, my) = (e.major_axis_endpoint.x, e.major_axis_endpoint.y);
                    let a = mx.hypot(my);
                    let b = a * e.minor_axis_ratio;
                    let rot = my.atan2(mx);
                    let mut sweep = e.end_angle - e.start_angle;
                    if sweep <= 0.0 {
                        sweep += 2.0 * PI;
                    }
                    let steps = ((sweep / (PI / 36.0)).ceil() as usize).max(2);
                    for k in 0..=steps {
                        let t = e.start_angle + sweep * k as f64 / steps as f64;
                        let t = if e.counter_clockwise { t } else { -t };
                        let (x, y) = (a * t.cos(), b * t.sin());
                        pts.push(([e.center.x + x * rot.cos() - y * rot.sin(), e.center.y + x * rot.sin() + y * rot.cos()], 0.0));
                    }
                }
                BoundaryEdge::Spline(s) => {
                    if s.fit_points.len() >= 2 {
                        pts.extend(s.fit_points.iter().map(|p| ([p.x, p.y], 0.0)));
                    } else {
                        pts.extend(s.control_points.iter().map(|p| ([p.x, p.y], 0.0)));
                    }
                }
            }
        }
        if pts.len() > 2 {
            let outer = path.flags.is_external() || path.flags.is_outermost();
            areas.push((ring_measure(&pts, true).1, outer));
        }
    }
    if areas.is_empty() {
        return 0.0;
    }
    // Without outer/inner flags: the largest path is the outline.
    if !areas.iter().any(|(_, o)| *o) {
        let max = areas.iter().map(|(a, _)| *a).fold(0.0, f64::max);
        let mut marked = false;
        for a in &mut areas {
            if !marked && a.0 == max {
                a.1 = true;
                marked = true;
            }
        }
    }
    let outer: f64 = areas.iter().filter(|(_, o)| *o).map(|(a, _)| a).sum();
    let inner: f64 = areas.iter().filter(|(_, o)| !*o).map(|(a, _)| a).sum();
    (outer - inner).max(0.0)
}

/// Quantities of the model space, or only of `selection`.
pub fn compute(doc: &CadDocument, unit_factor: f64, selection: Option<&HashSet<Handle>>) -> Quantities {
    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    let mut q = Quantities { from_selection: selection.is_some(), ..Default::default() };
    let f = unit_factor.max(1e-9);
    // Shapes seen per layer, to tell double-drawn copies. The object data
    // counts too: a copy carries the same, while two cables of a KLIC delivery
    // in one trench share the route but not their identifiers.
    let mut seen: std::collections::HashMap<(String, Vec<i64>, u64), Handle> = std::collections::HashMap::new();
    for e in doc.model_space_entities() {
        if matches!(e, EntityType::Viewport(_) | EntityType::AttributeDefinition(_) | EntityType::RasterImage(_)) {
            continue;
        }
        let c = e.common();
        if selection.is_some_and(|s| !s.contains(&c.handle)) {
            continue;
        }
        let infra = crate::library::infra::host::data(e);
        if infra.get("infra_primary").is_some_and(|v| v == "0") { continue; }
        let row_key = if let Some(unit)=infra.get("infra_unit") { format!("{}|{unit}",c.layer) } else { c.layer.clone() };
        let row = rows.entry(row_key).or_insert_with(|| {
            let mut r = Row { layer: c.layer.clone(), ..Default::default() };
            if let Ok(p) = nlcs::parse_layer(&c.layer) {
                r.status = p.status.code().into();
                r.discipline = p.discipline;
                r.group = p.group;
                r.object = p.object;
                r.element = p.element;
            }
            r
        });
        let (len, area, closed) = measure(e);
        if let Some(key) = shape_key(e, f) {
            let data = {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                format!("{:?}", c.extended_data).hash(&mut h);
                h.finish()
            };
            match seen.entry((c.layer.clone(), key, data)) {
                std::collections::hash_map::Entry::Occupied(first) => {
                    row.duplicates += 1;
                    row.duplicate_length += len / f;
                    q.duplicates.push((c.layer.clone(), *first.get(), c.handle));
                    continue;
                }
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert(c.handle);
                }
            }
        }
        if infra.get("infra_primary").is_some_and(|v| v == "1") {
            let unit = infra.get("infra_unit").map(String::as_str).unwrap_or("");
            let quantity = infra.get("infra_quantity").and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0);
            row.quantity_unit = unit.to_string();
            row.count += 1;
            if unit == "m" { row.length += quantity; }
            if unit == "m2" { row.area += quantity; row.closed += 1; }
            q.entities += 1;
            continue;
        }
        row.count += 1;
        row.length += len / f;
        row.area += area / (f * f);
        if closed {
            row.closed += 1;
        }
        q.entities += 1;
    }
    q.rows = rows.into_values().collect();
    // NLCS layers first, by main group and object.
    q.rows.sort_by(|a, b| (!a.nlcs(), &a.group, &a.object, &a.status, &a.element).cmp(&(!b.nlcs(), &b.group, &b.object, &b.status, &b.element)));
    q
}

/// Rows summed per object and status (G, S and A of one object together).
pub fn per_object(q: &Quantities) -> Vec<Row> {
    let mut m: BTreeMap<(String, String, String, String, String), Row> = BTreeMap::new();
    for r in q.rows.iter().filter(|r| r.nlcs() && !r.annotation() && r.element != "T" && r.element != "M" && r.element != "L") {
        let e = m.entry((r.group.clone(), r.object.clone(), r.status.clone(), r.discipline.clone(), r.quantity_unit.clone())).or_insert_with(|| Row {
            layer: format!("{}-{}-{}-{}", r.status, r.discipline, r.group, r.object),
            status: r.status.clone(),
            discipline: r.discipline.clone(),
            group: r.group.clone(),
            object: r.object.clone(),
            element: String::new(),
            ..Default::default()
        });
        e.count += r.count;
        if !r.quantity_unit.is_empty() { e.quantity_unit = r.quantity_unit.clone(); }
        e.length += r.length;
        e.area += r.area;
        e.closed += r.closed;
        e.duplicates += r.duplicates;
        e.duplicate_length += r.duplicate_length;
        if r.element == "A" {
            e.element = "A".into();
        }
    }
    m.into_values().collect()
}

/// CSV for Dutch spreadsheets: semicolons, decimal commas.
pub fn csv(q: &Quantities) -> String {
    let n = |v: f64| format!("{v:.2}").replace('.', ",");
    let mut out = String::from("Laag;Status;Discipline;Hoofdgroep;Object;Element;Aantal;Gesloten;Lengte (m);Oppervlakte (m2);Dubbel getekend (niet meegeteld);Dubbel (m)\n");
    for r in q.rows.iter().filter(|r| !r.annotation()) {
        out.push_str(&format!("{};{};{};{};{};{};{};{};{};{};{};{}\n", r.layer, r.status, r.discipline, r.group, r.object_label(), r.element, r.count, r.closed, n(r.length), n(r.area), r.duplicates, n(r.duplicate_length)));
    }
    out
}

/// A bill of quantities (hoeveelhedenstaat) as drawing entities with its
/// top-left corner at `at`: per object and status the main quantity. `k`
/// is drawing units per paper millimetre. Returns (layer, entities) pairs.
pub fn table(rows: &[Row], at: [f64; 2], k: f64, discipline: &str) -> Vec<(nlcs::LayerSpec, Vec<EntityType>)> {
    use ocs_plugin_api::host::codec::{Line, Text, Vector3};
    let obj = nlcs::object_by_name("AL", "TEKENBLAD_VERKLARING").or_else(|| nlcs::object_by_name("AL", "LEGENDA"));
    let Some(obj) = obj else { return Vec::new() };
    let lines = nlcs::layer(obj, nlcs::Status::B, discipline, nlcs::Element::G);
    let texts = nlcs::layer(obj, nlcs::Status::B, discipline, nlcs::Element::T);
    let cols = [0.0, 22.0, 102.0, 128.0, 150.0, 172.0];
    let width = 190.0;
    let row_h = 6.0;
    let mut g = Vec::new();
    let mut t = Vec::new();
    let p = |x: f64, y: f64| Vector3::new(at[0] + x * k, at[1] - y * k, 0.0);
    let mut text = |x: f64, y: f64, s: &str, mm: f64, right: bool| {
        let mut e = EntityType::Text(Text::with_value(s, p(x, y)).with_height(mm * k));
        if let EntityType::Text(tx) = &mut e {
            tx.horizontal_alignment = if right { ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Right } else { ocs_plugin_api::host::codec::entities::TextHorizontalAlignment::Left };
            tx.vertical_alignment = ocs_plugin_api::host::codec::entities::TextVerticalAlignment::Middle;
            tx.alignment_point = Some(p(x, y));
        }
        crate::tekenblad::text_pen(&mut e, mm);
        e.common_mut().layer = texts.name.clone();
        t.push(e);
    };
    text(0.0, -4.0, "HOEVEELHEDENSTAAT", 3.5, false);
    let header = ["code", "object", "status", "aantal", "lengte (m)", "opp. (m²)"];
    for (i, h) in header.iter().enumerate() {
        let right = i >= 3;
        text(if right { cols.get(i + 1).copied().unwrap_or(width) - 2.0 } else { cols[i] + 2.0 }, row_h / 2.0, h, 1.8, right);
    }
    let n = |v: f64| if v > 0.0 { format!("{v:.2}").replace('.', ",") } else { "–".into() };
    for (r, row) in rows.iter().enumerate() {
        let y = (r as f64 + 1.5) * row_h;
        text(cols[0] + 2.0, y, &format!("{}-{}", row.group, row.status), 1.8, false);
        let mut label = row.object_label();
        if label.chars().count() > 42 {
            label = label.chars().take(41).collect::<String>() + "…";
        }
        text(cols[1] + 2.0, y, &label, 1.8, false);
        text(cols[2] + 2.0, y, status_label(&row.status), 1.8, false);
        text(cols[4] - 2.0, y, &row.count.to_string(), 1.8, true);
        text(cols[5] - 2.0, y, &n(row.length), 1.8, true);
        text(width - 2.0, y, &n(row.area), 1.8, true);
    }
    let bottom = (rows.len() as f64 + 1.0) * row_h;
    let mut line = |x0: f64, y0: f64, x1: f64, y1: f64| {
        let mut e = EntityType::Line(Line::from_points(p(x0, y0), p(x1, y1)));
        e.common_mut().layer = lines.name.clone();
        g.push(e);
    };
    for y in [0.0, row_h, bottom] {
        line(0.0, y, width, y);
    }
    for x in cols.iter().chain(std::iter::once(&width)) {
        line(*x, 0.0, *x, bottom);
    }
    vec![(lines, g), (texts, t)]
}

pub fn status_label(code: &str) -> &'static str {
    match code {
        "B" => "bestaand",
        "N" => "nieuw",
        "T" => "tijdelijk",
        "V" => "te verwijderen",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocs_plugin_api::host::codec::{Circle, LwPolyline, Vector2, Vector3};

    fn square(bulge: f64) -> EntityType {
        let mut p = LwPolyline::new();
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)] {
            p.add_point_with_bulge(Vector2::new(x, y), 0.0);
        }
        p.vertices[0].bulge = bulge;
        p.is_closed = true;
        EntityType::LwPolyline(p)
    }

    #[test]
    fn lengths_and_areas() {
        let (l, a, closed) = measure(&square(0.0));
        assert!((l - 40.0).abs() < 1e-9 && (a - 100.0).abs() < 1e-9 && closed);
        // A half circle on the bottom side of this counter-clockwise square:
        // bulge +1 bends outward (+ π·5²/2), −1 inward (− π·5²/2).
        let (l, a, _) = measure(&square(1.0));
        assert!((a - (100.0 + PI * 12.5)).abs() < 1e-6, "{a}");
        assert!((l - (30.0 + PI * 5.0)).abs() < 1e-6, "{l}");
        let (_, a, _) = measure(&square(-1.0));
        assert!((a - (100.0 - PI * 12.5)).abs() < 1e-6, "{a}");
        let (l, a, _) = measure(&EntityType::Circle(Circle::from_center_radius(Vector3::new(0.0, 0.0, 0.0), 2.0)));
        assert!((l - 4.0 * PI).abs() < 1e-9 && (a - 4.0 * PI).abs() < 1e-9);
    }

    #[test]
    fn grouped_by_layer_in_metres() {
        let mut doc = CadDocument::new();
        // Two squares side by side (the same square twice would be a double-drawn copy).
        let mut beside = square(0.0);
        if let EntityType::LwPolyline(p) = &mut beside {
            p.vertices.iter_mut().for_each(|v| v.location.x += 20.0);
        }
        for (layer, e) in [("N-WE-VH-GESLOTENVERHARDING_ASFALT-G", square(0.0)), ("N-WE-VH-GESLOTENVERHARDING_ASFALT-G", beside), ("RANDOM", square(0.0))] {
            let mut e = e;
            e.common_mut().layer = layer.into();
            let _ = doc.add_entity(e);
        }
        // A mm drawing: 10 000 units = 10 m.
        let q = compute(&doc, 1000.0, None);
        assert_eq!(q.rows.len(), 2);
        let asfalt = &q.rows[0];
        assert_eq!((asfalt.status.as_str(), asfalt.group.as_str(), asfalt.count), ("N", "VH", 2));
        assert!((asfalt.area - 0.0002).abs() < 1e-9, "{}", asfalt.area);
        assert_eq!(asfalt.main().1, "m²");
        assert!(!q.rows[1].nlcs());
        assert!(csv(&q).lines().nth(1).unwrap().starts_with("N-WE-VH-GESLOTENVERHARDING_ASFALT-G;N;WE;VH;Geslotenverharding asfalt;G;2;2;"));
        assert_eq!(per_object(&q).len(), 1);
    }

    fn line(a: (f64, f64), b: (f64, f64), layer: &str) -> EntityType {
        let mut l = ocs_plugin_api::host::codec::Line::from_points(Vector3::new(a.0, a.1, 0.0), Vector3::new(b.0, b.1, 0.0));
        l.common.layer = layer.into();
        EntityType::Line(l)
    }

    #[test]
    fn double_drawn_lines_are_found_and_not_counted() {
        const L: &str = "N-WE-VH-BANDEN-G";
        let mut doc = CadDocument::new();
        let mut ring = square(0.0);
        ring.common_mut().layer = L.into();
        // The same square from another vertex and the other way round.
        let mut copy = LwPolyline::new();
        for (x, y) in [(10.0, 10.0), (10.0, 0.0), (0.0, 0.0), (0.0, 10.0)] {
            copy.add_point_with_bulge(Vector2::new(x, y), 0.0);
        }
        copy.is_closed = true;
        copy.common.layer = L.into();
        let mut open = LwPolyline::new();
        for (x, y) in [(30.0, 0.0), (20.0, 0.0)] {
            open.add_point_with_bulge(Vector2::new(x, y), 0.0);
        }
        open.common.layer = L.into();
        for e in [
            ring,
            EntityType::LwPolyline(copy),
            line((20.0, 0.0), (30.0, 0.0), L),
            line((30.0, 0.0), (20.0, 0.0), L),      // reversed copy
            EntityType::LwPolyline(open),            // the same line as a polyline
            line((20.0, 0.0), (30.0004, 0.0), L),   // within a millimetre
            line((20.0, 0.0), (30.0, 0.0), "OTHER"), // other layer: not a duplicate
            line((20.0, 0.0), (31.0, 0.0), L),      // another line
        ] {
            let _ = doc.add_entity(e);
        }
        // Two cables on one route with their own identifiers: both count.
        for id in ["KL1010.kabel_1", "KL1010.kabel_2"] {
            let mut cable = line((0.0, 50.0), (10.0, 50.0), L);
            cable.common_mut().extended_data.add_record(crate::bridge::object_data([("id", id.to_string())]));
            let _ = doc.add_entity(cable);
        }
        let q = compute(&doc, 1.0, None);
        let r = q.rows.iter().find(|r| r.layer == L).unwrap();
        assert_eq!((r.count, r.duplicates, q.duplicates.len()), (5, 4, 4), "{r:?}");
        assert!((r.length - (40.0 + 10.0 + 11.0 + 20.0)).abs() < 1e-6, "{}", r.length);
        assert!((r.duplicate_length - (40.0 + 10.0 + 10.0 + 10.0004)).abs() < 1e-3, "{}", r.duplicate_length);
        assert!((r.area - 100.0).abs() < 1e-9);
        assert!(q.duplicates_note().unwrap().contains(L));
        assert!(csv(&q).lines().find(|l| l.starts_with(L)).unwrap().ends_with(";4;70,00"));
        assert_eq!(per_object(&q)[0].duplicates, 4);
    }
}
