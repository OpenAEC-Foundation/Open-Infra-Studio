//! Point clouds for the civil tools: a TIN from the ground points of the AHN
//! point cloud or a file (LAS/LAZ/COPC, E57, PLY, PTS, XYZ), and ground
//! lines for the length profile and cross sections from a strip of points
//! along the axis.
//!
//! - **Input** ([`Invoer`]): the AHN COPC tiles of the project area or a
//!   selected area, or a file. Points stream: the AHN and COPC files by
//!   box, other files in one pass; nothing holds the whole cloud.
//! - **Classes** ([`Klassen`]): ASPRS classes taken as ground (2 ground and
//!   9 water by default). A cloud without class 2 points, or `KLASSEN=AUTO`,
//!   goes through the ground filter ([`grondfilter`], SMRF).
//! - **Thinning** ([`dunnen`]): one point per cell (`CEL=`, 0.5 m) while the
//!   cloud streams, then greedy insertion until the TIN lies within `TOL=`
//!   of every point or the triangle budget (`MAX=`) is spent.
//! - **Output**: the existing [`terrein::Tin`] path (3D faces on
//!   OG-TRIANGULATIE, status B or N), so contours, analysis, volumes and
//!   profiles work on it as on any TIN.

/// A message in the interface language: `trf!("nl {}", "en {}", args…)`.
macro_rules! trf {
    ($nl:literal, $en:literal $(, $a:expr)* $(,)?) => {
        if crate::lang::current() == crate::lang::Lang::Nl { format!($nl $(, $a)*) } else { format!($en $(, $a)*) }
    };
}
pub(crate) use trf;

pub mod dunnen;
pub mod grondfilter;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use ocs_plugin_api::host::codec::EntityType;

use self::dunnen::{CellGrid, P3};
use self::grondfilter::Smrf;
use super::api::{self, Args};
use super::{terrein, CivielOptions, Ctx, Out};
use crate::bridge;
use crate::gis::crs::{self, XY};
use crate::gis::pointcloud::{self, Edition, Pass, Sample};
use crate::gis::site::Site;
use crate::hoogte::nap;
use crate::lang::tr;

// ── Arguments ───────────────────────────────────────────────────────────────

/// Where the points come from.
#[derive(Clone, Debug, PartialEq)]
pub enum Invoer {
    /// The AHN point cloud (COPC tiles of Het Waterschapshuis).
    Ahn(Edition),
    /// A point cloud file: LAS/LAZ (COPC too), E57, PLY, PTS, XYZ.
    Bestand(PathBuf),
}

impl Invoer {
    /// `BESTAND=<pad>`, else `EDITIE=AHN5|AHN4` (AHN5 by default).
    pub fn from_args(args: &Args) -> Result<Invoer, String> {
        if let Some(b) = args.str("BESTAND") {
            return Invoer::file(b);
        }
        match args.str("EDITIE").map(|s| s.to_ascii_uppercase()).as_deref() {
            None | Some("AHN5") | Some("5") => Ok(Invoer::Ahn(Edition::Ahn5)),
            Some("AHN4") | Some("4") => Ok(Invoer::Ahn(Edition::Ahn4)),
            Some(_) => Err(tr("EDITIE: AHN5 of AHN4", "EDITIE: AHN5 or AHN4").into()),
        }
    }

    /// A file argument (quotes and surrounding spaces taken off).
    pub fn file(p: &str) -> Result<Invoer, String> {
        let path = PathBuf::from(p.trim().trim_matches('"'));
        if !path.exists() {
            return Err(trf!("bestand {} bestaat niet", "file {} does not exist", path.display()));
        }
        if !pointcloud::is_point_file(&path) {
            return Err(trf!("{}: geen puntenwolk (LAS, LAZ, E57, PLY, PTS, XYZ)", "{}: not a point cloud (LAS, LAZ, E57, PLY, PTS, XYZ)", path.display()));
        }
        Ok(Invoer::Bestand(path))
    }

    pub fn label(&self) -> String {
        match self {
            Invoer::Ahn(e) => trf!("{}-puntenwolk", "{} point cloud", e.short()),
            Invoer::Bestand(p) => p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }
}

/// Which ASPRS classes count as ground.
#[derive(Clone, Debug, PartialEq)]
pub enum Klassen {
    Lijst(Vec<u8>),
    /// Find the ground with the filter, whatever the classes say.
    Auto,
}

impl Klassen {
    /// ASPRS 2 (ground) and 9 (water), as AHN classifies them.
    pub const DEFAULT: [u8; 2] = [2, 9];

    /// `KLASSEN=2;9` (or `2,9`), or `KLASSEN=AUTO`.
    pub fn from_args(args: &Args) -> Result<Klassen, String> {
        match args.str("KLASSEN") {
            None => Ok(Klassen::Lijst(Klassen::DEFAULT.to_vec())),
            Some(v) if v.eq_ignore_ascii_case("AUTO") || v.eq_ignore_ascii_case("FILTER") => Ok(Klassen::Auto),
            Some(v) => {
                let l: Result<Vec<u8>, _> = v.split([';', ',']).filter(|t| !t.trim().is_empty()).map(|t| t.trim().parse::<u8>()).collect();
                match l {
                    Ok(l) if !l.is_empty() => Ok(Klassen::Lijst(l)),
                    _ => Err(tr("KLASSEN: ASPRS-klassen als 2;9, of AUTO (grondfilter)", "KLASSEN: ASPRS classes such as 2;9, or AUTO (ground filter)").into()),
                }
            }
        }
    }

    /// A point of class `c` is wanted (without a class: no, the filter decides).
    fn wanted(&self, c: Option<u8>) -> bool {
        match self {
            Klassen::Auto => true,
            Klassen::Lijst(l) => c.is_some_and(|c| l.contains(&c)),
        }
    }

    /// Whether the ground filter takes over: asked for, or ground wanted
    /// from a cloud that has no ground class.
    fn filter(&self, ground_class_points: u64) -> bool {
        match self {
            Klassen::Auto => true,
            Klassen::Lijst(l) => l.contains(&2) && ground_class_points == 0,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Klassen::Auto => tr("grondfilter", "ground filter").into(),
            Klassen::Lijst(l) => trf!("klassen {}", "classes {}", l.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(";")),
        }
    }
}

/// The ground filter's settings: `HELLING=` (slope still ground, rise over
/// run) and `VENSTER=` (largest window, m: about half the largest building).
fn smrf_args(args: &Args) -> Result<Smrf, String> {
    let mut s = Smrf { slope: 0.3, ..Smrf::default() };
    if let Some(h) = args.f64("HELLING")? {
        s.slope = h.clamp(0.01, 5.0);
    }
    if let Some(w) = args.f64("VENSTER")? {
        s.window = w.clamp(2.0, 200.0);
    }
    Ok(s)
}

/// ASPRS noise classes, never used: 7 (low point) and 18 (high noise).
fn noise(c: Option<u8>) -> bool {
    matches!(c, Some(7) | Some(18))
}

// ── A point source in the drawing ───────────────────────────────────────────

/// A point source opened for the drawing: its CRS and the way to drawing
/// coordinates and units.
pub struct Bron {
    pub invoer: Invoer,
    crs: String,
    crs_from: &'static str,
    site: Option<Site>,
    f: f64,
    /// Source CRS → project CRS (none: the same, or no project location).
    to_site: Option<crs::Transformer>,
    copc: bool,
}

impl Bron {
    pub fn open(ctx: &Ctx, invoer: &Invoer) -> Result<Bron, String> {
        match invoer {
            Invoer::Ahn(_) => {
                let site = ctx.site.clone().ok_or(tr("stel eerst de projectlocatie in (GIS → Locatie): de AHN-puntenwolk hangt eraan", "set the project location first (GIS → Location): the AHN point cloud depends on it"))?;
                if !crs::in_netherlands(site.lon, site.lat) {
                    return Err(tr("de AHN-puntenwolk is alleen beschikbaar in Nederland; geef BESTAND=<las/laz/e57/ply/pts/xyz>", "the AHN point cloud covers the Netherlands only; give BESTAND=<las/laz/e57/ply/pts/xyz>").into());
                }
                let to = transformer("EPSG:28992", &site.crs)?;
                Ok(Bron { invoer: invoer.clone(), crs: "EPSG:28992".into(), crs_from: "AHN", site: Some(site), f: ctx.f, to_site: to, copc: true })
            }
            Invoer::Bestand(path) => {
                let copc = pointcloud::is_copc(path);
                match &ctx.site {
                    Some(site) => {
                        let (crs, from) = crate::gis::rasterbestand::point_file_crs(site, path);
                        let to = transformer(&crs, &site.crs)?;
                        Ok(Bron { invoer: invoer.clone(), crs, crs_from: from, site: Some(site.clone()), f: ctx.f, to_site: to, copc })
                    }
                    // Without a project location the coordinates are taken as they are.
                    None => Ok(Bron { invoer: invoer.clone(), crs: String::new(), crs_from: "", site: None, f: ctx.f, to_site: None, copc }),
                }
            }
        }
    }

    /// AHN and COPC files read a box without reading the rest.
    pub fn indexed(&self) -> bool {
        self.copc
    }

    /// Source coordinates → drawing coordinates and units.
    fn to_drawing(&self, s: [f64; 3]) -> Option<P3> {
        let q = match &self.to_site {
            Some(t) => t.apply([s[0], s[1]]).ok()?,
            None => [s[0], s[1]],
        };
        let d = match &self.site {
            Some(site) => site.to_drawing(q, self.f),
            None => [q[0] * self.f, q[1] * self.f],
        };
        Some([d[0], d[1], s[2] * self.f])
    }

    /// A drawing box in the source's CRS (its envelope, a metre wider).
    fn source_bbox(&self, r: [f64; 4]) -> Result<[f64; 4], String> {
        let Some(site) = &self.site else {
            return Ok([r[0] / self.f - 1.0, r[1] / self.f - 1.0, r[2] / self.f + 1.0, r[3] / self.f + 1.0]);
        };
        let a = site.from_drawing([r[0], r[1]], self.f);
        let b = site.from_drawing([r[2], r[3]], self.f);
        let bb = [a[0], a[1], b[0], b[1]];
        let e = if self.crs.is_empty() || site.crs.eq_ignore_ascii_case(&self.crs) { bb } else { crate::gis::site::envelope(bb, &crs::Transformer::new(&site.crs, &self.crs)?)? };
        Ok([e[0] - 1.0, e[1] - 1.0, e[2] + 1.0, e[3] + 1.0])
    }

    /// Stream the points inside a drawing box (all of them without one), in
    /// drawing coordinates with their class. `spacing` (m) is the COPC level
    /// of detail; 0 reads everything.
    pub fn stream(&self, rect: Option<[f64; 4]>, spacing: f64, max_tiles: usize, progress: &dyn Fn(String), visit: &mut dyn FnMut(P3, Option<u8>) -> bool) -> Result<Pass, String> {
        let sb = rect.map(|r| self.source_bbox(r)).transpose()?;
        let mut v = |s: &Sample| -> bool {
            let Some(d) = self.to_drawing(s.xyz) else { return true };
            if let Some(r) = rect {
                if d[0] < r[0] || d[0] > r[2] || d[1] < r[1] || d[1] > r[3] {
                    return true;
                }
            }
            visit(d, s.class)
        };
        match &self.invoer {
            Invoer::Ahn(ed) => pointcloud::stream_ahn(sb.ok_or(tr("geen gebied voor de AHN-puntenwolk", "no area for the AHN point cloud"))?, *ed, spacing, max_tiles, progress, &mut v),
            Invoer::Bestand(p) => pointcloud::stream_file(p, sb, spacing, progress, &mut v),
        }
    }

    pub fn label(&self) -> String {
        self.invoer.label()
    }

    /// The CRS and where it came from, for reports.
    pub fn crs_label(&self) -> String {
        if self.crs.is_empty() { tr("zonder projectlocatie", "without a project location").into() } else { format!("{} ({})", self.crs, crs_source(self.crs_from)) }
    }

    pub fn attribution(&self) -> Option<String> {
        match &self.invoer {
            Invoer::Ahn(e) => Some(trf!("{} puntenwolk (Het Waterschapshuis, CC0)", "{} point cloud (Het Waterschapshuis, CC0)", e.short())),
            Invoer::Bestand(_) => None,
        }
    }
}

/// A transformation between two CRSs, none when they are the same.
fn transformer(from: &str, to: &str) -> Result<Option<crs::Transformer>, String> {
    if from.eq_ignore_ascii_case(to) { Ok(None) } else { crs::Transformer::new(from, to).map(Some) }
}

/// The area a command works in, in drawing coordinates: `GEBIED=` (two
/// corners or a polygon, project coordinates), else the selected closed
/// polyline (or circle). `None` when neither is given.
pub fn area(ctx: &Ctx, gebied: &Option<Vec<XY>>) -> Result<Option<Vec<XY>>, String> {
    if let Some(g) = gebied {
        let d: Vec<XY> = g.iter().map(|p| ctx.to_drawing(*p)).collect();
        return match d.len() {
            2 => {
                let (x0, x1) = (d[0][0].min(d[1][0]), d[0][0].max(d[1][0]));
                let (y0, y1) = (d[0][1].min(d[1][1]), d[0][1].max(d[1][1]));
                Ok(Some(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]))
            }
            n if n >= 3 => Ok(Some(d)),
            _ => Err(tr("GEBIED: twee hoeken x,y;x,y of een veelhoek x,y;x,y;x,y;…", "GEBIED: two corners x,y;x,y or a polygon x,y;x,y;x,y;…").into()),
        };
    }
    let sel = bridge::selection().unwrap_or_default();
    if sel.is_empty() {
        return Ok(None);
    }
    for e in bridge::entities(&sel[..sel.len().min(50)])?.into_iter().flatten() {
        if let Some(r) = closed_ring(&e) {
            return Ok(Some(r));
        }
    }
    Ok(None)
}

/// The plan outline of a closed polyline or a circle.
pub fn closed_ring(e: &EntityType) -> Option<Vec<XY>> {
    match e {
        EntityType::LwPolyline(l) if l.is_closed && l.vertices.len() >= 3 => Some(l.vertices.iter().map(|v| [v.location.x, v.location.y]).collect()),
        EntityType::Polyline3D(p) if p.flags.closed && p.vertices.len() >= 3 => Some(p.vertices.iter().map(|v| [v.position.x, v.position.y]).collect()),
        EntityType::Circle(c) => Some((0..48).map(|i| {
            let a = i as f64 / 48.0 * std::f64::consts::TAU;
            [c.center.x + c.radius * a.cos(), c.center.y + c.radius * a.sin()]
        }).collect()),
        _ => None,
    }
}

/// Where a CRS came from, for reports: the file, a .prj beside it, AHN or the project.
pub fn crs_source(from: &str) -> &'static str {
    match from {
        "bestand" => tr("bestand", "file"),
        ".prj" => ".prj",
        "AHN" => "AHN",
        _ => tr("project", "project"),
    }
}

pub fn bounds(ring: &[XY]) -> [f64; 4] {
    ring.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, p| [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])])
}

// ── TIN from a point cloud ──────────────────────────────────────────────────

/// Most grid cells while thinning (about 120 MB): a larger area gets a
/// larger cell.
const MAX_CELLS: usize = 2_000_000;

/// `CIV_TIN BRON=PUNTENWOLK`: what to triangulate.
#[derive(Clone, Debug, PartialEq)]
pub struct TinVraag {
    pub invoer: Invoer,
    pub klassen: Klassen,
    /// Thinning cell (m).
    pub cel: f64,
    /// Vertical tolerance of the greedy thinning (m).
    pub tol: f64,
    pub max_driehoeken: usize,
    /// Triangles farther than this from any point are left out (m).
    pub gat: f64,
    /// Area in project coordinates (two corners or a polygon).
    pub gebied: Option<Vec<XY>>,
    pub smrf: Smrf,
}

impl TinVraag {
    /// The arguments besides the settings (`CEL=` is one of those).
    pub const ARGS: &'static [&'static str] = &["BRON", "BESTAND", "EDITIE", "KLASSEN", "TOL", "MAX", "GAT", "GEBIED", "HELLING", "VENSTER"];

    pub fn from_args(args: &Args) -> Result<TinVraag, String> {
        Ok(TinVraag {
            invoer: Invoer::from_args(args)?,
            klassen: Klassen::from_args(args)?,
            cel: args.f64("CEL")?.unwrap_or(0.5).clamp(0.05, 100.0),
            tol: args.f64("TOL")?.unwrap_or(0.10).max(0.0),
            max_driehoeken: args.f64("MAX")?.map(|m| m as usize).unwrap_or(200_000).clamp(1_000, 2_000_000),
            gat: args.f64("GAT")?.unwrap_or(2.0).max(0.0),
            gebied: args.points("GEBIED")?,
            smrf: smrf_args(args)?,
        })
    }
}

/// The ground points of an area, thinned per cell: classified ground, or
/// what the ground filter finds. Drawing coordinates.
pub struct Grond {
    pub points: Vec<P3>,
    /// Points of the source inside the area (noise left out).
    pub read: u64,
    pub filtered: bool,
    /// The thinning cell in the end (m).
    pub cell: f64,
    pub pass: Pass,
}

/// Stream an area once into a cell grid and keep its ground.
pub fn ground_points(
    bron: &Bron,
    rect: Option<[f64; 4]>,
    polygon: Option<&[XY]>,
    klassen: &Klassen,
    cell: f64,
    smrf: &Smrf,
    max_tiles: usize,
    progress: &dyn Fn(String),
) -> Result<Grond, String> {
    let f = bron.f;
    let mut grid = CellGrid::new(cell * f, MAX_CELLS, true);
    let (mut read, mut class2) = (0u64, 0u64);
    let pass = bron.stream(rect, cell / 2.0, max_tiles, progress, &mut |p, c| {
        if noise(c) {
            return true;
        }
        if let Some(poly) = polygon {
            if !crate::gis::vector::point_in_ring([p[0], p[1]], poly) {
                return true;
            }
        }
        read += 1;
        if c == Some(2) {
            class2 += 1;
        }
        grid.add(p, klassen.wanted(c));
        true
    })?;
    let filtered = klassen.filter(class2) && !grid.is_empty();
    let points = if filtered {
        progress(trf!("Grondfilter (SMRF) over {} cellen…", "Ground filter (SMRF) over {} cells…", grid.len()));
        let lows = grid.lowest();
        let m: Vec<P3> = lows.iter().map(|p| [p[0] / f, p[1] / f, p[2] / f]).collect();
        let s = Smrf { cell: smrf.cell.max(grid.cell() / f), ..*smrf };
        let mask = grondfilter::ground(&m, &s)?;
        let (lows, m): (Vec<P3>, Vec<P3>) = lows.into_iter().zip(m).zip(mask).filter(|(_, g)| *g).map(|(pm, _)| pm).unzip();
        // Low points on walls and cars pass the filter's threshold: spikes go.
        let keep = grondfilter::without_spikes(&m, grid.cell() / f, grondfilter::SPIKE);
        lows.into_iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| p).collect()
    } else {
        grid.kept()
    };
    Ok(Grond { points, read, filtered, cell: grid.cell() / f, pass })
}

/// `CIV_TIN BRON=PUNTENWOLK`: a TIN from the ground points of a point cloud,
/// on OG-TRIANGULATIE like every terrain (status from the settings).
pub fn create_tin(opt: &CivielOptions, v: &TinVraag, progress: &dyn Fn(String)) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let f = ctx.f;
    let bron = Bron::open(&ctx, &v.invoer)?;
    let polygon = area(&ctx, &v.gebied)?;
    let rect = match (&polygon, &v.invoer) {
        (Some(p), _) => Some(bounds(p)),
        (None, Invoer::Ahn(_)) => ctx.site.as_ref().map(|s| s.drawing_rect(f)),
        // A file without an area: all of it.
        (None, Invoer::Bestand(_)) => None,
    };
    // The cell: CEL, larger when the area would need more cells than fit.
    let cel = match rect {
        Some(r) => v.cel.max(((r[2] - r[0]) * (r[3] - r[1]) / (f * f) / MAX_CELLS as f64).sqrt()),
        None => v.cel,
    };
    progress(trf!("{} lezen…", "Reading {}…", bron.label()));
    let g = ground_points(&bron, rect, polygon.as_deref(), &v.klassen, cel, &v.smrf, 64, progress)?;
    if g.points.len() < 3 {
        return Err(trf!(
            "{} grondpunten van {} in het gebied: te weinig voor een terrein (kies andere KLASSEN of KLASSEN=AUTO)",
            "{} ground points of {} in the area: too few for a terrain (choose other KLASSEN or KLASSEN=AUTO)",
            g.points.len(),
            bron.label()
        ));
    }
    progress(trf!("Driehoeken kiezen uit {} grondpunten…", "Choosing triangles from {} ground points…", g.points.len()));
    let gap = if v.gat > 0.0 { v.gat.max(3.0 * g.cell) * f } else { 0.0 };
    let t = dunnen::greedy_tin(&g.points, v.tol * f, v.max_driehoeken, gap)?;
    let faces: Vec<[P3; 3]> = t.triangles.iter().map(|tr| tr.map(|i| t.points[i])).collect();
    let tin = terrein::Tin::from_faces(&faces).ok_or(tr("geen driehoeken", "no triangles"))?;
    let name = terrein::next_name();
    let how = if g.filtered { tr("grondfilter (SMRF)", "ground filter (SMRF)").to_string() } else { v.klassen.label() };
    let source = trf!("{}, {}, cel {} m, tol {} m", "{}, {}, cell {} m, tol {} m", bron.label(), how, fmt(g.cell), fmt(v.tol));
    let mut out = Out::default();
    let (faces_l, meshes) = terrein::draw(&ctx, &tin, &name, &source, 0.0, &mut out)?;
    let (zmin, zmax) = tin.range();
    api::note("terrein", name.clone());
    api::note("driehoeken", tin.triangles.len());
    api::note("punten", tin.points.len());
    api::note("laag", faces_l.name.clone());
    api::note("hoogte_min", zmin / f);
    api::note("hoogte_max", zmax / f);
    api::note("bron", bron.label());
    api::note("crs", bron.crs_label());
    api::note("punten_gelezen", g.read);
    api::note("punten_grond", g.points.len());
    api::note("grondfilter", g.filtered);
    api::note("klassen", v.klassen.label());
    api::note("cel_m", g.cell);
    api::note("tol_m", v.tol);
    api::note("max_fout_m", t.max_error / f);
    api::note("begrensd", t.capped);
    if g.pass.tiles > 0 {
        api::note("tegels", g.pass.tiles);
        api::note("opgehaald_mb", g.pass.bytes as f64 / 1e6);
    }
    progress(tr("Terrein tekenen…", "Drawing the terrain…").into());
    let handles = out.write_handles(&name)?;
    let written = handles.len();
    let status = faces_l.name.chars().next().unwrap_or('B').to_string();
    bridge::index_add("tin", &status, &handles[..meshes.min(handles.len())], false);
    if let Some(a) = bron.attribution() {
        bridge::note_source(&a);
    }
    let capped = if t.capped {
        trf!(", begrensd op {} driehoeken: grootste afwijking {} m", ", capped at {} triangles: largest deviation {} m", v.max_driehoeken, fmt((t.max_error / f * 100.0).round() / 100.0))
    } else {
        String::new()
    };
    Ok(trf!(
        "{}: {} van {} grondpunten, {} driehoeken, {} tot {} m ({}{}; {}, cel {} m, tol {} m{}) — {} elementen op {}.",
        "{}: {} of {} ground points, {} triangles, {} to {} m ({}{}; {}, cell {} m, tol {} m{}) — {} entities on {}.",
        name,
        tin.points.len(),
        g.points.len(),
        tin.triangles.len(),
        nap(zmin / f, 2),
        nap(zmax / f, 2),
        bron.label(),
        if g.filtered { tr(", grond gevonden met het grondfilter", ", ground found by the ground filter") } else { "" },
        how,
        fmt(g.cell),
        fmt(v.tol),
        capped,
        written,
        faces_l.name
    ))
}


/// A number in the interface's style: "0,5" in Dutch, "0.5" in English.
pub fn fmt(v: f64) -> String {
    let s = format!("{:.3}", v);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if crate::lang::current() == crate::lang::Lang::Nl { s.replace('.', ",") } else { s.to_string() }
}

// ── Ground lines for profiles ───────────────────────────────────────────────

/// Ground heights for the length profile and cross sections from a point
/// cloud (`BRON=PUNTENWOLK`): the median height of the ground points within
/// a strip of `STROOK=` m across, around each station.
#[derive(Clone, Debug, PartialEq)]
pub struct Grondbron {
    pub invoer: Invoer,
    pub klassen: Klassen,
    /// Width of the strip (m): the window around a station is a disc of
    /// this diameter.
    pub strook: f64,
    pub smrf: Smrf,
}

impl Grondbron {
    pub const ARGS: &'static [&'static str] = &["BRON", "BESTAND", "EDITIE", "KLASSEN", "STROOK", "HELLING", "VENSTER"];

    /// Asked for with `BRON=PUNTENWOLK`, `BESTAND=` or `STROOK=`; `BRON=TIN`
    /// (or none of these) keeps the TIN in the drawing or AHN.
    pub fn from_args(args: &Args) -> Result<Option<Grondbron>, String> {
        let wanted = match args.str("BRON").map(|s| s.to_ascii_uppercase()).as_deref() {
            Some("PUNTENWOLK") | Some("WOLK") => true,
            Some("TIN") | Some("AHN") | Some("AUTO") => false,
            Some(_) => return Err(tr("BRON: PUNTENWOLK (of TIN)", "BRON: PUNTENWOLK (or TIN)").into()),
            None => args.str("BESTAND").is_some() || args.str("STROOK").is_some(),
        };
        if !wanted {
            return Ok(None);
        }
        Ok(Some(Grondbron {
            invoer: Invoer::from_args(args)?,
            klassen: Klassen::from_args(args)?,
            strook: args.f64("STROOK")?.unwrap_or(1.0).clamp(0.1, 50.0),
            smrf: smrf_args(args)?,
        }))
    }
}

/// Blocks of points loaded as the stations ask for them (m).
const BLOCK: f64 = 100.0;
/// Margin around a block for the ground filter (m): its largest window.
const FILTER_MARGIN: f64 = 20.0;

/// The ground points along the axis, loaded block by block (AHN and COPC
/// files), or once for the whole file (other formats).
pub struct Strook {
    bron: Bron,
    g: Grondbron,
    f: f64,
    bins: HashMap<(i64, i64), Vec<P3>>,
    bin: f64,
    blocks: HashSet<(i64, i64)>,
    whole: bool,
    err: Option<String>,
    filtered: bool,
    points: usize,
}

impl Strook {
    pub fn new(ctx: &Ctx, g: &Grondbron) -> Result<Strook, String> {
        let bron = Bron::open(ctx, &g.invoer)?;
        Ok(Strook { bron, g: g.clone(), f: ctx.f, bins: HashMap::new(), bin: g.strook * ctx.f, blocks: HashSet::new(), whole: false, err: None, filtered: false, points: 0 })
    }

    fn key(&self, p: XY) -> (i64, i64) {
        ((p[0] / self.bin).floor() as i64, (p[1] / self.bin).floor() as i64)
    }

    /// Load the ground points of a drawing box (all of the file: `None`).
    fn load(&mut self, rect: Option<[f64; 4]>) -> Result<(), String> {
        let f = self.f;
        let may_filter = !matches!(&self.g.klassen, Klassen::Lijst(l) if !l.contains(&2));
        let m = if may_filter { FILTER_MARGIN * f } else { 0.0 };
        let wide = rect.map(|r| [r[0] - m, r[1] - m, r[2] + m, r[3] + m]);
        let cell = (self.g.strook / 4.0).clamp(0.05, 0.5);
        let g = ground_points(&self.bron, wide, None, &self.g.klassen, cell, &self.g.smrf, 4, &|_| {})?;
        self.filtered |= g.filtered;
        for p in g.points {
            if rect.is_none_or(|r| p[0] >= r[0] && p[0] < r[2] && p[1] >= r[1] && p[1] < r[3]) {
                let k = self.key([p[0], p[1]]);
                self.bins.entry(k).or_default().push(p);
                self.points += 1;
            }
        }
        Ok(())
    }

    /// The median ground height (m) within half a strip of `p` (drawing).
    pub fn height(&mut self, p: XY) -> Option<f64> {
        if self.err.is_some() {
            return None;
        }
        let half = self.bin / 2.0;
        if self.bron.indexed() {
            let b = BLOCK * self.f;
            let (i0, i1) = (((p[0] - half) / b).floor() as i64, ((p[0] + half) / b).floor() as i64);
            let (j0, j1) = (((p[1] - half) / b).floor() as i64, ((p[1] + half) / b).floor() as i64);
            for i in i0..=i1 {
                for j in j0..=j1 {
                    if self.blocks.insert((i, j)) {
                        let r = [i as f64 * b, j as f64 * b, (i + 1) as f64 * b, (j + 1) as f64 * b];
                        if let Err(e) = self.load(Some(r)) {
                            self.err = Some(e);
                            return None;
                        }
                    }
                }
            }
        } else if !self.whole {
            self.whole = true;
            if let Err(e) = self.load(None) {
                self.err = Some(e);
                return None;
            }
        }
        let (ki, kj) = self.key(p);
        let mut zs: Vec<f64> = Vec::new();
        for i in ki - 1..=ki + 1 {
            for j in kj - 1..=kj + 1 {
                if let Some(v) = self.bins.get(&(i, j)) {
                    zs.extend(v.iter().filter(|q| (q[0] - p[0]).hypot(q[1] - p[1]) <= half).map(|q| q[2]));
                }
            }
        }
        if zs.is_empty() {
            return None;
        }
        zs.sort_by(f64::total_cmp);
        let n = zs.len();
        let med = if n % 2 == 1 { zs[n / 2] } else { (zs[n / 2 - 1] + zs[n / 2]) / 2.0 };
        Some(med / self.f)
    }

    pub fn label(&self) -> String {
        trf!(
            "puntenwolk {} (strook {} m{})",
            "point cloud {} (strip {} m{})",
            self.bron.label(),
            fmt(self.g.strook),
            if self.filtered { tr(", grondfilter", ", ground filter") } else { "" }
        )
    }

    pub fn attribution(&self) -> Option<String> {
        self.bron.attribution()
    }

    pub fn error(&self) -> Option<String> {
        self.err.clone().or_else(|| (self.points == 0).then(|| trf!("geen grondpunten van {} langs de as", "no ground points of {} along the alignment", self.bron.label())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        let a = Args::parse("BRON=PUNTENWOLK KLASSEN=2;9;26 CEL=0,25 TOL=0.05 MAX=50000 GEBIED=1,2;3,4 HELLING=0.5");
        let v = TinVraag::from_args(&a).unwrap();
        assert_eq!(v.invoer, Invoer::Ahn(Edition::Ahn5));
        assert_eq!(v.klassen, Klassen::Lijst(vec![2, 9, 26]));
        assert_eq!((v.cel, v.tol, v.max_driehoeken), (0.25, 0.05, 50_000));
        assert_eq!(v.gebied, Some(vec![[1.0, 2.0], [3.0, 4.0]]));
        assert_eq!(v.smrf.slope, 0.5);
        assert_eq!(Klassen::from_args(&Args::parse("KLASSEN=auto")).unwrap(), Klassen::Auto);
        assert!(Klassen::from_args(&Args::parse("KLASSEN=grond")).is_err());
        assert!(Invoer::from_args(&Args::parse("BESTAND=/bestaat/niet.laz")).is_err());
        assert_eq!(Invoer::from_args(&Args::parse("EDITIE=ahn4")).unwrap(), Invoer::Ahn(Edition::Ahn4));
        // Profiles: only when asked.
        assert_eq!(Grondbron::from_args(&Args::parse("AS=E9E")).unwrap(), None);
        let g = Grondbron::from_args(&Args::parse("AS=E9E BRON=PUNTENWOLK STROOK=2")).unwrap().unwrap();
        assert_eq!(g.strook, 2.0);
        assert!(Grondbron::from_args(&Args::parse("BRON=TIN")).unwrap().is_none());
        assert!(fmt(0.5) == "0,5" || fmt(0.5) == "0.5");
        assert_eq!(fmt(2.0), "2");
    }

    /// The synthetic unclassified scan (tests/data/puntenwolk): the ground
    /// filter finds the ground under the building and the trees, and the
    /// thinned TIN follows the true ground.
    #[test]
    fn a_tin_from_the_unclassified_test_scan() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/puntenwolk/onbewerkt.las");
        let k = Klassen::Lijst(Klassen::DEFAULT.to_vec());
        let mut grid = CellGrid::new(0.5, 1_000_000, true);
        let mut class2 = 0;
        pointcloud::stream_file(&path, None, 0.0, &|_| {}, &mut |s| {
            if s.class == Some(2) {
                class2 += 1;
            }
            grid.add(s.xyz, k.wanted(s.class));
            true
        })
        .unwrap();
        assert!(grid.kept().is_empty(), "no point is class 2 or 9");
        assert!(k.filter(class2), "no class 2: the filter takes over");
        let lows = grid.lowest();
        let mask = grondfilter::ground(&lows, &Smrf { slope: 0.3, ..Smrf::default() }).unwrap();
        let ground: Vec<P3> = lows.iter().zip(&mask).filter(|(_, g)| **g).map(|(p, _)| *p).collect();
        let keep = grondfilter::without_spikes(&ground, 0.5, grondfilter::SPIKE);
        let ground: Vec<P3> = ground.into_iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| p).collect();
        let g = dunnen::greedy_tin(&ground, 0.10, 200_000, 2.0).unwrap();
        let faces: Vec<[P3; 3]> = g.triangles.iter().map(|t| t.map(|i| g.points[i])).collect();
        let tin = terrein::Tin::from_faces(&faces).unwrap();
        let (x0, y0) = (105_400.0, 425_300.0);
        let truth = |x: f64, y: f64| 0.8 + 0.01 * (x - x0) + 0.3 * ((x - x0) / 12.0).sin() * ((y - y0) / 9.0).cos();
        let off: Vec<(f64, P3)> = tin.points.iter().map(|p| ((p[2] - truth(p[0], p[1])).abs(), *p)).collect();
        let worst = off.iter().cloned().fold((0.0, [0.0; 3]), |a, b| if b.0 > a.0 { b } else { a });
        // Points low on a wall (within the spike limit) may stay.
        assert!(worst.0 < 0.45, "a TIN vertex lies {:.2} m off the ground: {:?}", worst.0, worst.1);
        // Away from the wall feet the TIN lies on the ground.
        let wall = |p: &P3| {
            let (x, y) = (p[0] - x0, p[1] - y0);
            (x > 29.5 && x < 50.5 && y > 19.5 && y < 35.5) && !(x > 30.5 && x < 49.5 && y > 20.5 && y < 34.5)
        };
        let open: Vec<&(f64, P3)> = off.iter().filter(|o| !wall(&o.1)).collect();
        let close = open.iter().filter(|o| o.0 < 0.1).count() as f64 / open.len() as f64;
        assert!(close > 0.98, "{close}");
        // Fewer vertices than ground cells: flat parts are thinned.
        assert!(tin.points.len() < ground.len() / 2, "{} of {}", tin.points.len(), ground.len());
        // The building (20 × 15 m) is a hole; the open ground is covered.
        assert_eq!(tin.height([x0 + 40.0, y0 + 27.5]), None);
        let z = tin.height([x0 + 10.0, y0 + 10.0]).unwrap();
        assert!((z - truth(x0 + 10.0, y0 + 10.0)).abs() < 0.15);
        // Under a tree crown the ground is there too.
        let z = tin.height([x0 + 65.0, y0 + 12.0]).unwrap();
        assert!((z - truth(x0 + 65.0, y0 + 12.0)).abs() < 0.2, "{z}");
    }

    #[test]
    fn classes_and_the_filter() {
        let k = Klassen::Lijst(vec![2, 9]);
        assert!(k.wanted(Some(2)) && k.wanted(Some(9)) && !k.wanted(Some(6)) && !k.wanted(None));
        assert!(k.filter(0) && !k.filter(10));
        assert!(!Klassen::Lijst(vec![9]).filter(0));
        assert!(Klassen::Auto.filter(10));
        assert!(noise(Some(7)) && noise(Some(18)) && !noise(Some(2)) && !noise(None));
    }
}
