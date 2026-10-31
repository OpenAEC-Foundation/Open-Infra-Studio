//! `CIV_MESH`: a 3D mesh (not a 2.5D TIN) of the points inside a selected
//! closed polyline or a box, for bridges, quay walls, tunnels and other
//! existing objects.
//!
//! The mesher is the closed mesher of OpenAEC's Open Pointcloud Studio
//! (`pointcloud-core`, LGPL-3.0-or-later): the points become oriented surface
//! elements, a signed distance to the surface is found on a voxel lattice,
//! and dual contouring takes the surface out (after Ju et al. 2002). It is
//! closed wherever the scan has data or a gap narrower than `GAT=`.
//!
//! The mesh goes on the NLCS layer of the structure (`SOORT=`, KW-KUNSTWERK
//! by default) as light MESH entities in batches under the host's message
//! limit, with object data: source, points, triangles, voxel.

use pointcloud_core::region_source::{RegionSource, SourceTransform};
use pointcloud_core::surfels::SurfelSource;
use pointcloud_core::{ClosedMeshConfig, IndexedPoint, MeshOrientation, Point};

use super::api::{self, Args};
use super::puntenwolk::{self, trf, Bron, Invoer};
use super::{record, CivielOptions, Ctx, Out};
use crate::gis::crs::XY;
use crate::lang::tr;
use crate::nlcs::Element;

/// Which side the surface faces where the points do not tell.
#[derive(Clone, Debug, PartialEq)]
pub enum Zijde {
    /// Measured from above (airborne, AHN): up, and one fixed direction
    /// for upright faces.
    Boven,
    /// The middle of the area: right for a tunnel or a room seen from inside.
    Midden,
    /// Towards a point (project coordinates and m NAP): the scanner side.
    Naar([f64; 3]),
}

/// What `CIV_MESH` meshes.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshVraag {
    pub invoer: Invoer,
    /// Area in project coordinates (two corners or a polygon); without it
    /// the selected closed polyline.
    pub gebied: Option<Vec<XY>>,
    /// Height range (m).
    pub zmin: Option<f64>,
    pub zmax: Option<f64>,
    /// Classes to mesh; `None`: every class but noise.
    pub klassen: Option<Vec<u8>>,
    /// Voxel edge (m), 0.005–0.5; `None` follows the point spacing.
    pub voxel: Option<f64>,
    /// Widest gap closed (m), at most 3.2.
    pub gat: f64,
    pub max_driehoeken: usize,
    /// NLCS object: kunstwerk, brug, viaduct, tunnel, sluis, steiger, pijler, landhoofd.
    pub soort: String,
    pub zijde: Option<Zijde>,
    pub naam: Option<String>,
}

/// The structures `SOORT=` knows and their NLCS objects.
const SOORTEN: &[(&str, &str)] = &[
    ("kunstwerk", "KUNSTWERK"),
    ("brug", "KUNSTWERK_BRUG"),
    ("viaduct", "KUNSTWERK_VIADUCT"),
    ("tunnel", "KUNSTWERK_TUNNEL"),
    ("sluis", "KUNSTWERK_SLUIS"),
    ("steiger", "KUNSTWERK_STEIGER"),
    ("pijler", "KUNSTWERK_PIJLER"),
    ("landhoofd", "KUNSTWERK_LANDHOOFD"),
    // A quay wall has no NLCS object of its own: the structure.
    ("kademuur", "KUNSTWERK"),
];

/// Most points a mesh job takes; more are thinned evenly.
const MAX_POINTS: usize = 12_000_000;
/// Largest area side (m).
const MAX_SIDE: f64 = 600.0;

impl MeshVraag {
    pub const ARGS: &'static [&'static str] = &["BESTAND", "EDITIE", "GEBIED", "ZMIN", "ZMAX", "KLASSEN", "VOXEL", "GAT", "MAX", "SOORT", "ZIJDE", "NAAM"];

    pub fn from_args(args: &Args) -> Result<MeshVraag, String> {
        let soort = args.str("SOORT").unwrap_or("kunstwerk").to_ascii_lowercase();
        if !SOORTEN.iter().any(|(k, _)| *k == soort) {
            return Err(format!("SOORT: {}", SOORTEN.iter().map(|(k, _)| *k).collect::<Vec<_>>().join(", ")));
        }
        let klassen = match args.str("KLASSEN") {
            None => None,
            Some(v) if v.eq_ignore_ascii_case("ALLE") => None,
            Some(v) => Some(v.split([';', ',']).filter(|t| !t.trim().is_empty()).map(|t| t.trim().parse::<u8>().map_err(|_| tr("KLASSEN: ASPRS-klassen als 6;26, of ALLE", "KLASSEN: ASPRS classes such as 6;26, or ALLE").to_string())).collect::<Result<Vec<u8>, String>>()?),
        };
        let zijde = match args.str("ZIJDE").map(|s| s.to_ascii_uppercase()) {
            None => None,
            Some(s) if s == "BOVEN" => Some(Zijde::Boven),
            Some(s) if s == "MIDDEN" || s == "BINNEN" => Some(Zijde::Midden),
            Some(s) => {
                let bad = || tr("ZIJDE: BOVEN, MIDDEN of een punt x,y,z", "ZIJDE: BOVEN, MIDDEN or a point x,y,z").to_string();
                let v: Vec<f64> = s.split(',').map(|t| t.trim().parse::<f64>()).collect::<Result<_, _>>().map_err(|_| bad())?;
                if v.len() != 3 {
                    return Err(bad());
                }
                Some(Zijde::Naar([v[0], v[1], v[2]]))
            }
        };
        Ok(MeshVraag {
            invoer: puntenwolk::Invoer::from_args(args)?,
            gebied: args.points("GEBIED")?,
            zmin: args.f64("ZMIN")?,
            zmax: args.f64("ZMAX")?,
            klassen,
            voxel: args.f64("VOXEL")?.map(|v| v.clamp(0.005, 0.5)),
            gat: args.f64("GAT")?.unwrap_or(1.0).clamp(0.0, 3.2),
            max_driehoeken: args.f64("MAX")?.map(|m| m as usize).unwrap_or(400_000).clamp(1_000, 3_000_000),
            soort,
            zijde,
            naam: args.str("NAAM").map(str::to_string),
        })
    }
}

/// The voxel for a point spacing: about two and a half spacings, within
/// the mesher's range.
pub fn voxel_for(points: usize, plan_area_m2: f64) -> f64 {
    let spacing = (plan_area_m2 / points.max(1) as f64).sqrt();
    (2.5 * spacing).clamp(0.02, 0.5)
}

/// The point spacing of a cloud (m): the median distance to the nearest
/// neighbour of up to 2000 points spread over it. `guess` sizes the search
/// grid (about the spacing).
pub fn spacing(pts: &[[f64; 3]], guess: f64) -> Option<f64> {
    if pts.len() < 2 {
        return None;
    }
    let g = guess.max(1e-3);
    let key = |p: &[f64; 3]| ((p[0] / g).floor() as i64, (p[1] / g).floor() as i64, (p[2] / g).floor() as i64);
    let mut grid: std::collections::HashMap<(i64, i64, i64), Vec<usize>> = std::collections::HashMap::new();
    for (i, p) in pts.iter().enumerate() {
        grid.entry(key(p)).or_default().push(i);
    }
    let every = (pts.len() / 2000).max(1);
    let mut d: Vec<f64> = Vec::new();
    for i in (0..pts.len()).step_by(every) {
        let p = pts[i];
        let (a, b, c) = key(&p);
        let mut best = f64::INFINITY;
        for x in a - 1..=a + 1 {
            for y in b - 1..=b + 1 {
                for z in c - 1..=c + 1 {
                    for &j in grid.get(&(x, y, z)).into_iter().flatten() {
                        if j != i {
                            let q = pts[j];
                            let dd = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt();
                            if dd > 1e-6 {
                                best = best.min(dd);
                            }
                        }
                    }
                }
            }
        }
        if best.is_finite() {
            d.push(best);
        }
    }
    if d.is_empty() {
        return None;
    }
    d.sort_by(f64::total_cmp);
    Some(d[d.len() / 2])
}

/// The closed mesh of points (metres, scene coordinates), with a coarser
/// voxel when the first try gives more triangles than allowed.
pub fn mesh_points(pts: &[[f64; 3]], voxel: f64, gap: f64, side: MeshOrientation, max_triangles: usize) -> Result<(pointcloud_core::MeshGeometry, pointcloud_core::ClosedMeshReport), String> {
    let resident: Vec<IndexedPoint> = pts.iter().enumerate().map(|(i, p)| IndexedPoint { point: Point { xyz: *p, rgb: None, intensity: None, classification: None }, ordinal: i as u64 }).collect();
    let mut voxel = voxel.clamp(0.005, 0.5);
    loop {
        let source = SurfelSource::without_stations(RegionSource::resident(&resident, SourceTransform::default()));
        let config = ClosedMeshConfig {
            voxel: Some(voxel),
            max_hole: gap.min(32.0 * voxel).min(3.2),
            orientation: side,
            max_triangles,
            max_vertices: max_triangles.max(3),
            // Two threads: the plugin shares the machine with the drawing.
            threads: 2,
            deviation_samples: 20_000,
            ..ClosedMeshConfig::default()
        };
        match pointcloud_core::mesh_closed(&[source], None, &|_, _, _| true, &config, &mut |_| Ok(())) {
            Ok(r) => return Ok(r),
            Err(e) => {
                let msg = e.to_string();
                // Too many triangles: a coarser voxel, up to the mesher's largest.
                let too_many = msg.contains("would exceed");
                if too_many && voxel < 0.5 {
                    voxel = (voxel * 1.6).min(0.5);
                    continue;
                }
                return Err(trf!("mesh: {}", "mesh: {}", msg));
            }
        }
    }
}

/// `CIV_MESH`: the closed mesh of the points in an area.
pub fn create(opt: &CivielOptions, v: &MeshVraag, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let f = ctx.f;
    let polygon = puntenwolk::area(&ctx, &v.gebied)?.ok_or(tr("selecteer een gesloten polylijn rond het object, of geef GEBIED=x,y;x,y", "select a closed polyline around the object, or give GEBIED=x,y;x,y"))?;
    let rect = puntenwolk::bounds(&polygon);
    let (w, h) = ((rect[2] - rect[0]) / f, (rect[3] - rect[1]) / f);
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(trf!("het gebied is {:.0} × {:.0} m; een 3D-mesh kan tot {:.0} m: kies een kleiner gebied", "the area is {:.0} × {:.0} m; a 3D mesh takes up to {:.0} m: choose a smaller area", w, h, MAX_SIDE));
    }
    let bron = Bron::open(&ctx, &v.invoer)?;
    // Points in metres around the middle of the area (the mesher's lattice
    // and precision like small numbers).
    let mid = [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0];
    let (zlo, zhi) = (v.zmin.map(|z| z * f), v.zmax.map(|z| z * f));
    let mut pts: Vec<[f64; 3]> = Vec::new();
    let mut stride = 1u64;
    let mut seen = 0u64;
    progress(trf!("{} lezen…", "Reading {}…", bron.label()));
    let pass = bron.stream(Some(rect), 0.0, 9, progress, &mut |p, c| {
        if matches!(c, Some(7) | Some(18)) {
            return true;
        }
        if let (Some(k), Some(c)) = (&v.klassen, c) {
            if !k.contains(&c) {
                return true;
            }
        }
        if zlo.is_some_and(|z| p[2] < z) || zhi.is_some_and(|z| p[2] > z) {
            return true;
        }
        if !crate::gis::vector::point_in_ring([p[0], p[1]], &polygon) {
            return true;
        }
        seen += 1;
        if seen % stride != 0 {
            return true;
        }
        pts.push([(p[0] - mid[0]) / f, (p[1] - mid[1]) / f, p[2] / f]);
        if pts.len() >= MAX_POINTS {
            let mut i = 0;
            pts.retain(|_| {
                i += 1;
                i % 2 == 1
            });
            stride *= 2;
        }
        true
    })?;
    if pts.len() < 50 {
        return Err(trf!("{} punten in het gebied: te weinig voor een mesh", "{} points in the area: too few for a mesh", pts.len()));
    }
    let plan = polygon_area(&polygon) / (f * f);
    // The voxel: twice the measured point spacing (a 3D object holds more
    // points per square metre of plan than a surface).
    let voxel = v.voxel.unwrap_or_else(|| {
        let guess = (plan / pts.len() as f64).sqrt();
        spacing(&pts, guess.max(0.01)).map(|s| (2.0 * s).clamp(0.02, 0.5)).unwrap_or_else(|| voxel_for(pts.len(), plan))
    });
    let side = match &v.zijde {
        Some(Zijde::Boven) => MeshOrientation::Upward,
        Some(Zijde::Midden) => MeshOrientation::Automatic,
        Some(Zijde::Naar(p)) => {
            let d = ctx.to_drawing([p[0], p[1]]);
            MeshOrientation::Towards([(d[0] - mid[0]) / f, (d[1] - mid[1]) / f, p[2]])
        }
        // AHN is measured from above.
        None => match v.invoer {
            Invoer::Ahn(_) => MeshOrientation::Upward,
            Invoer::Bestand(_) => MeshOrientation::Automatic,
        },
    };
    progress(trf!("3D-mesh van {} punten (voxel {} m)…", "3D mesh of {} points (voxel {} m)…", pts.len(), voxel));
    let (mesh, report) = mesh_points(&pts, voxel, v.gat, side, v.max_driehoeken)?;
    let object = SOORTEN.iter().find(|(k, _)| *k == v.soort).map(|(_, o)| *o).unwrap_or("KUNSTWERK");
    let layer = ctx.layer("KW", object, Element::G)?;
    // A name that tells meshes apart: the middle of the area (project coordinates).
    let name = v.naam.clone().unwrap_or_else(|| {
        let c = ctx.from_drawing(mid);
        format!("{} {:.0},{:.0}", tr("Mesh", "Mesh"), c[0], c[1])
    });
    let closed = report.topology.open_edges == 0 && report.topology.non_manifold_edges == 0;
    let d = record(&[
        ("mesh", name.clone()),
        ("naam", name.clone()),
        ("bron", bron.label()),
        ("punten", pts.len().to_string()),
        ("driehoeken", mesh.triangles.len().to_string()),
        ("voxel_m", format!("{:.3}", report.voxel)),
        ("gesloten", if closed { "ja".into() } else { format!("nee ({})", report.topology.open_edges) }),
        ("afwijking_p95_m", format!("{:.3}", report.deviation.p95)),
    ]);
    let mut b = super::model3d::MeshBuilder::default();
    for t in &mesh.triangles {
        let [a, bb, c] = t.map(|i| {
            let q = mesh.vertices[i as usize];
            [q[0] * f + mid[0], q[1] * f + mid[1], q[2] * f]
        });
        b.tri(a, bb, c);
    }
    let ents = b.finish(&layer.name, None, Some(&d));
    let n_ent = ents.len();
    let mut out = Out::default();
    out.put(&layer, ents);
    progress(tr("Mesh tekenen…", "Drawing the mesh…").into());
    let handles = out.write_handles(&name)?;
    api::note("mesh", name.clone());
    api::note("laag", layer.name.clone());
    api::note("punten", pts.len());
    api::note("punten_gelezen", pass.read);
    api::note("driehoeken", mesh.triangles.len());
    api::note("hoekpunten", mesh.vertices.len());
    api::note("voxel_m", report.voxel);
    api::note("gesloten", closed);
    api::note("open_randen", report.topology.open_edges);
    api::note("delen", report.topology.components);
    api::note("afwijking_gem_m", report.deviation.mean);
    api::note("afwijking_p95_m", report.deviation.p95);
    api::note("handles", handles.iter().map(|h| api::handle_text(*h)).collect::<Vec<_>>());
    if let Some(a) = bron.attribution() {
        crate::bridge::note_source(&a);
    }
    let state = if closed { tr("gesloten", "closed").to_string() } else { trf!("{} open randen", "{} open edges", report.topology.open_edges) };
    Ok(trf!(
        "{}: {} driehoeken uit {} punten (voxel {} m, {}; 95 % van de punten binnen {} m) — {} mesh(es) op {}.",
        "{}: {} triangles from {} points (voxel {} m, {}; 95 % of the points within {} m) — {} mesh(es) on {}.",
        name,
        mesh.triangles.len(),
        pts.len(),
        puntenwolk::fmt((report.voxel * 100.0).round() / 100.0),
        state,
        puntenwolk::fmt((report.deviation.p95 * 100.0).round() / 100.0),
        n_ent,
        layer.name
    ))
}

/// Plan area of a ring (shoelace).
fn polygon_area(r: &[XY]) -> f64 {
    let mut a = 0.0;
    for i in 0..r.len() {
        let (p, q) = (r[i], r[(i + 1) % r.len()]);
        a += p[0] * q[1] - q[0] * p[1];
    }
    (a / 2.0).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box of 6 × 4 × 3 m sampled on every face every 5 cm: a closed mesh
    /// that keeps the box's size.
    #[test]
    fn a_scanned_box_gives_a_closed_mesh() {
        let (a, b, c) = (6.0, 4.0, 3.0);
        let step = 0.05;
        let mut pts = Vec::new();
        let grid = |u: f64, v: f64| -> Vec<(f64, f64)> {
            let mut out = Vec::new();
            let mut x = 0.0;
            while x <= u + 1e-9 {
                let mut y = 0.0;
                while y <= v + 1e-9 {
                    out.push((x, y));
                    y += step;
                }
                x += step;
            }
            out
        };
        for (x, y) in grid(a, b) {
            pts.push([x, y, 0.0]);
            pts.push([x, y, c]);
        }
        for (x, z) in grid(a, c) {
            pts.push([x, 0.0, z]);
            pts.push([x, b, z]);
        }
        for (y, z) in grid(b, c) {
            pts.push([0.0, y, z]);
            pts.push([a, y, z]);
        }
        let (m, r) = mesh_points(&pts, 0.1, 0.3, MeshOrientation::Towards([3.0, 2.0, 1.5]), 200_000).unwrap();
        assert!(m.triangles.len() > 12, "{} triangles", m.triangles.len());
        let lo = m.vertices.iter().fold([f64::MAX; 3], |l, v| [l[0].min(v[0]), l[1].min(v[1]), l[2].min(v[2])]);
        let hi = m.vertices.iter().fold([f64::MIN; 3], |h, v| [h[0].max(v[0]), h[1].max(v[1]), h[2].max(v[2])]);
        for k in 0..3 {
            assert!(lo[k] > -0.15 && hi[k] < [a, b, c][k] + 0.15, "axis {k}: {lo:?} – {hi:?}");
        }
        assert!(r.deviation.p95 < 0.05, "{:?}", r.deviation);
        assert_eq!(r.topology.open_edges, 0, "{:?}", r.topology);
    }

    /// The IFC export takes a mesh of several MESH entities as one civil
    /// element, and a point cloud TIN as terrain.
    #[test]
    fn meshes_and_point_cloud_tins_reach_the_ifc_export() {
        use crate::nlcs::{self, Status};
        use ocs_plugin_api::host::codec::entities::{Mesh, MeshFace};
        use ocs_plugin_api::host::codec::{CadDocument, EntityType, Vector3};
        let layer = |g: &str, o: &str| nlcs::layer(nlcs::object_by_name(g, o).unwrap(), Status::B, "XX", Element::G).name;
        let tri = |z: f64, layer: &str, data: &[(&str, String)]| {
            let mut m = Mesh::new();
            m.vertices = vec![Vector3::new(0.0, 0.0, z), Vector3::new(10.0, 0.0, z), Vector3::new(0.0, 10.0, z + 1.0)];
            m.faces = vec![MeshFace::new(vec![0, 1, 2])];
            let mut e = EntityType::Mesh(m);
            e.common_mut().layer = layer.to_string();
            e.common_mut().extended_data.add_record(record(data));
            e
        };
        let mut doc = CadDocument::new();
        let bridge = [("mesh", "Brug 1".to_string()), ("naam", "Brug 1".to_string()), ("bron", "AHN5-puntenwolk".to_string())];
        let tin = [("terrein", "Terrein 1".to_string()), ("bron", "AHN5-puntenwolk, klassen 2;9, cel 0,5 m, tol 0,1 m".to_string())];
        doc.add_entity(tri(4.0, &layer("KW", "KUNSTWERK_BRUG"), &bridge)).unwrap();
        doc.add_entity(tri(5.0, &layer("KW", "KUNSTWERK_BRUG"), &bridge)).unwrap();
        doc.add_entity(tri(0.0, &layer("OG", "TRIANGULATIE"), &tin)).unwrap();
        doc.add_entity(tri(0.5, &layer("OG", "TRIANGULATIE"), &tin)).unwrap();
        let src = crate::ifc::Source { doc: &doc, f: 1.0, site: None, name: "puntenwolk.dwg".into() };
        let mut report = crate::ifc::Report::default();
        let out = crate::ifc::export(&src, &crate::ifc::Options::default(), Vec::new(), &mut report).unwrap();
        let ifc = String::from_utf8(out).unwrap();
        assert_eq!(report.classes.get("IfcCivilElement"), Some(&1), "{:?}", report.classes);
        assert_eq!(report.classes.get("IfcGeographicElement"), Some(&1), "{:?}", report.classes);
        assert!(ifc.contains("'Brug 1'"));
        assert!(ifc.contains("IFCGEOGRAPHICELEMENT(") && ifc.contains(".TERRAIN."));
    }

    /// The footbridge of tests/data/puntenwolk (sampled every 0.12 m): the
    /// spacing is found, and the mesh at twice it stays close to the points.
    #[test]
    fn the_test_bridge_meshes() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/puntenwolk/brug.ply");
        let mut pts = Vec::new();
        crate::gis::pointcloud::stream_file(&path, None, 0.0, &|_| {}, &mut |s| {
            pts.push([s.xyz[0] - 105_420.0, s.xyz[1] - 425_272.0, s.xyz[2]]);
            true
        })
        .unwrap();
        let s = spacing(&pts, 0.3).unwrap();
        assert!((s - 0.12).abs() < 0.03, "spacing {s}");
        let (m, r) = mesh_points(&pts, 2.0 * s, 1.0, MeshOrientation::Automatic, 400_000).unwrap();
        assert!(m.triangles.len() > 500);
        assert!(r.deviation.p95 < 0.15, "{:?}", r.deviation);
    }

    #[test]
    fn voxels_follow_the_spacing() {
        // AHN: about 20 points a square metre.
        assert!((voxel_for(20_000, 1_000.0) - 0.559).abs() < 0.06 || voxel_for(20_000, 1_000.0) == 0.5);
        // A terrestrial scan: 2 500 points a square metre.
        assert!((voxel_for(2_500_000, 1_000.0) - 0.05).abs() < 1e-9);
        assert_eq!(voxel_for(10, 1.0e6), 0.5);
        assert!((polygon_area(&[[0.0, 0.0], [2.0, 0.0], [2.0, 3.0], [0.0, 3.0]]) - 6.0).abs() < 1e-12);
    }
}
