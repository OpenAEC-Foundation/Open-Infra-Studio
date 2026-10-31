//! Civil tools after Hakan Seven's FreeCAD Road workbench: survey points
//! (GeoPoints), terrain (TIN), alignments with stationing, length and cross
//! sections, LandXML. Drawn on NLCS layers like the rest of Open Infra.

pub mod api;
pub mod as_profiel;
pub mod corridor;
pub mod landmeten;
pub mod landxml;
pub mod model3d;
pub mod ondergrond;
pub mod terrein;
// Plan production: sheets and labels.
pub mod bladen;
pub mod labels;

use std::sync::{Mutex, OnceLock};

use ocs_plugin_api::host::codec::{EntityType, Vector3};
use ocs_plugin_api::host::ExtendedDataRecord;

use crate::basisdata::NlcsOptions;
use crate::bridge;
use crate::gis::ahn::{self, Model};
use crate::gis::crs::{self, XY};
use crate::gis::site::Site;
use crate::nlcs::{self, Element, LayerSpec};

#[derive(Clone, Debug)]
pub struct CivielOptions {
    pub nlcs: NlcsOptions,
    /// Column order of point files.
    pub format: landmeten::Format,
    /// Point labels: number, height, code.
    pub point_labels: bool,
    /// TIN: longest triangle edge (m); longer ones (concave borders) drop.
    pub max_edge: f64,
    /// TIN from AHN without a selection: grid spacing (m).
    pub ahn_spacing: f64,
    /// Contours on the TIN (m); every `major_every`-th is a major line.
    pub interval: f64,
    pub major_every: u32,
    /// Stationing ticks and labels along an alignment (m).
    pub tick: f64,
    pub label_every: f64,
    /// Length profile: sample distance (m) and vertical exaggeration.
    pub step: f64,
    pub exaggeration: f64,
    /// Cross sections: distance along the alignment, widths left and right,
    /// vertical exaggeration, sections per row.
    pub section_every: f64,
    pub left: f64,
    pub right: f64,
    pub section_exaggeration: f64,
    pub per_row: usize,
    /// Offset lines beside the axis (m, right > 0), as offset alignments.
    pub offsets: Vec<f64>,
    /// Design profile: length of the vertical curves (m).
    pub vertical_curve: f64,
    /// Height analysis: number of bands.
    pub bands: usize,
    /// Earthwork: grid cell (m) and the level for "to level" volumes.
    pub volume_cell: f64,
    pub level: f64,
    /// Corridor: assembly name, section distance (m), section views.
    pub assembly: String,
    pub corridor_every: f64,
    pub corridor_views: bool,
}

impl Default for CivielOptions {
    fn default() -> Self {
        // Road's defaults: sections every 10 m, 20 m both sides; contours 1 and 5 m.
        Self {
            nlcs: NlcsOptions::default(),
            format: landmeten::Format::Auto,
            point_labels: true,
            max_edge: 100.0,
            ahn_spacing: 5.0,
            interval: 0.25,
            major_every: 4,
            tick: 10.0,
            label_every: 100.0,
            step: 1.0,
            exaggeration: 10.0,
            section_every: 20.0,
            left: 20.0,
            right: 20.0,
            section_exaggeration: 2.0,
            per_row: 4,
            offsets: vec![-3.0, 3.0],
            vertical_curve: 50.0,
            bands: 8,
            volume_cell: 1.0,
            level: 0.0,
            assembly: "weg".into(),
            corridor_every: 10.0,
            corridor_views: false,
        }
    }
}

/// The settings the window last used; ribbon commands use them too.
pub fn settings() -> CivielOptions {
    store().lock().unwrap().clone()
}

pub fn set_settings(o: CivielOptions) {
    *store().lock().unwrap() = o;
}

fn store() -> &'static Mutex<CivielOptions> {
    static S: OnceLock<Mutex<CivielOptions>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(CivielOptions::default()))
}

/// The drawing these tools work in: units, scale, project location.
pub struct Ctx {
    /// Drawing units per metre.
    pub f: f64,
    /// Paper millimetre in drawing units (scale / 1000 × f).
    pub k: f64,
    pub site: Option<Site>,
    pub opt: CivielOptions,
}

impl Ctx {
    pub fn new(opt: &CivielOptions) -> Result<Ctx, String> {
        let t = bridge::target().ok_or("geen tekening actief — klik in Open CAD Studio op een knop van het GIS-tabblad")?;
        Ok(Ctx { f: t.unit_factor, k: opt.nlcs.scale / 1000.0 * t.unit_factor, site: bridge::site(), opt: opt.clone() })
    }

    /// Project coordinates (metres in the site's CRS) → drawing. Without a
    /// project location the coordinates are taken as they are.
    pub fn to_drawing(&self, p: XY) -> XY {
        match &self.site {
            Some(s) => s.to_drawing(p, self.f),
            None => [p[0] * self.f, p[1] * self.f],
        }
    }

    pub fn from_drawing(&self, p: XY) -> XY {
        match &self.site {
            Some(s) => s.from_drawing(p, self.f),
            None => [p[0] / self.f, p[1] / self.f],
        }
    }

    pub fn layer(&self, group: &str, name: &str, el: Element) -> Result<LayerSpec, String> {
        let obj = nlcs::object_by_name(group, name).ok_or_else(|| format!("NLCS-object {group} {name} ontbreekt"))?;
        Ok(nlcs::layer(obj, self.opt.nlcs.status, &self.opt.nlcs.discipline, el))
    }

    /// A text at paper size `mm` in the NLCS text class pen.
    pub fn text(&self, at: XY, value: &str, mm: f64, rotation: f64, spec: &LayerSpec, data: Option<&ExtendedDataRecord>) -> EntityType {
        let mut t = crate::emit::text(at, value, mm * self.k, rotation, &spec.name, data);
        crate::tekenblad::text_pen(&mut t, mm);
        t
    }
}

/// What a civil command reads: the whole drawing when Open CAD Studio can
/// share it, otherwise the Open Infra objects of these index kinds ("tin",
/// "as", "lengteprofiel", "ontwerpprofiel", "corridor", "punten"), the
/// selection and `extra`, fetched by handle — drawings of any size.
pub fn document(needs: &[&str], extra: &[ocs_plugin_api::host::codec::Handle]) -> Result<ocs_plugin_api::host::codec::CadDocument, String> {
    if let Some(doc) = bridge::view() {
        return Ok(doc);
    }
    let mut small: Vec<ocs_plugin_api::host::codec::Handle> = extra.to_vec();
    small.extend(bridge::selection().unwrap_or_default());
    let mut meshes = Vec::new();
    for (kind, _, hs) in bridge::index_read() {
        if needs.contains(&kind.as_str()) {
            if kind == "tin" { meshes.extend(hs) } else { small.extend(hs) }
        }
    }
    let mut seen = std::collections::HashSet::new();
    small.retain(|h| seen.insert(*h));
    meshes.retain(|h| seen.insert(*h));
    // Terrain meshes are large: a few at a time.
    let mut doc = bridge::partial_document(&small, 500)?;
    for chunk in meshes.chunks(4) {
        for e in bridge::entities(chunk)?.into_iter().flatten() {
            let mut e = e;
            e.common_mut().owner_handle = ocs_plugin_api::host::codec::Handle::NULL;
            if !doc.layers.iter().any(|l| l.name == e.common().layer) {
                let _ = doc.layers.add(ocs_plugin_api::host::codec::tables::Layer::new(e.common().layer.as_str()));
            }
            let _ = doc.add_entity(e);
        }
    }
    Ok(doc)
}

/// Entities per NLCS layer, written as one undo step.
#[derive(Default)]
pub struct Out {
    layers: Vec<(LayerSpec, Vec<EntityType>)>,
}

impl Out {
    pub fn put(&mut self, spec: &LayerSpec, e: impl IntoIterator<Item = EntityType>) {
        match self.layers.iter_mut().find(|(s, _)| s.name == spec.name) {
            Some((_, v)) => v.extend(e),
            None => self.layers.push((spec.clone(), e.into_iter().collect())),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|(_, e)| e.is_empty())
    }

    pub fn write(self, label: &str) -> Result<usize, String> {
        self.write_handles(label).map(|h| h.len())
    }

    /// Write and return the new handles, in the order the entities were put
    /// (layer by layer, as first used).
    pub fn write_handles(self, label: &str) -> Result<Vec<ocs_plugin_api::host::codec::Handle>, String> {
        let mut all = Vec::new();
        let mut first_data = None;
        for (spec, e) in self.layers {
            bridge::ensure_nlcs_layer(&spec);
            if first_data.is_none() {
                first_data = e.iter().find_map(|x| x.common().extended_data.get_record(bridge::XDATA_APP).cloned());
            }
            all.extend(e);
        }
        let handles = bridge::add_entities_handles(label, all)?;
        // The host registers the XDATA application only through a record write.
        if let (Some(h), Some(r)) = (handles.first(), first_data) {
            bridge::register_object_data(*h, r);
        }
        api::note("elementen", handles.len());
        Ok(handles)
    }
}

/// Object data (XDATA "OPENINFRA") from key/value pairs.
pub fn record(pairs: &[(&str, String)]) -> ExtendedDataRecord {
    bridge::object_data(pairs.iter().map(|(k, v)| (*k, v.clone())))
}

/// One value of an entity's object data.
pub fn data_value(e: &EntityType, key: &str) -> Option<String> {
    let r = e.common().extended_data.get_record(bridge::XDATA_APP)?;
    r.values.iter().find_map(|v| match v {
        ocs_plugin_api::host::XDataValue::String(s) => s.strip_prefix(key).and_then(|r| r.strip_prefix('=')).map(str::to_string),
        _ => None,
    })
}

pub fn v3(p: XY, z: f64) -> Vector3 {
    Vector3::new(p[0], p[1], z)
}

/// Station the Dutch way: "0+120", or "0+120,50" between whole metres.
pub fn station(m: f64) -> String {
    let neg = m < -1e-9;
    let m = m.abs();
    let km = (m / 1000.0).floor();
    let rest = m - km * 1000.0;
    let rest_s = if (rest - rest.round()).abs() < 0.005 { format!("{:03.0}", rest.round()) } else { format!("{:06.2}", rest).replace('.', ",") };
    // 999,996 rounds up into the next kilometre.
    let (km, rest_s) = if rest_s.starts_with("1000") { (km + 1.0, "000".to_string()) } else { (km, rest_s) };
    format!("{}{}+{}", if neg { "-" } else { "" }, km, rest_s)
}

/// 1, 2 or 5 × 10ⁿ at or above `v`.
pub fn nice_step(v: f64) -> f64 {
    let v = v.max(1e-3);
    let p = 10f64.powf(v.log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * p).find(|s| *s >= v * 0.999).unwrap_or(10.0 * p)
}

/// Where heights come from when the drawing has no TIN: AHN for now.
/// Kept in this one place so a worldwide source (`gis::terrain`) can take
/// over without touching the tools.
pub struct Hoogtebron {
    /// AHN in the Netherlands, Terrain Tiles elsewhere ([`crate::gis::terrain`]).
    src: crate::gis::terrain::Source,
    ground: crate::gis::terrain::Sampler,
    /// AHN's surface model (DSM); outside the Netherlands the ground serves.
    surface: Option<ahn::Sampler>,
    site: Site,
    f: f64,
}

pub fn hoogtebron(ctx: &Ctx) -> Result<Hoogtebron, String> {
    let site = ctx.site.clone().ok_or_else(|| crate::lang::tr("stel eerst de projectlocatie in (GIS → Locatie): daar hangen de hoogtes aan", "set the project location first (GIS → Location): the heights depend on it").to_string())?;
    let src = crate::gis::terrain::Source::for_site(&site, Model::Dtm);
    let ground = src.sampler(site.lat, 1.0)?;
    let surface = src.is_ahn().then(|| ahn::Sampler::new(Model::Dsm, 1.0));
    Ok(Hoogtebron { src, ground, surface, site, f: ctx.f })
}

impl Hoogtebron {
    /// A drawing point in the height source's CRS.
    fn in_src(&self, p: XY) -> Option<XY> {
        let q = self.site.from_drawing(p, self.f);
        if self.site.crs == self.src.crs { Some(q) } else { crs::transform(&self.site.crs, &self.src.crs, q).ok() }
    }

    /// Ground height (m) at a drawing point.
    pub fn height(&mut self, p: XY) -> Option<f64> {
        let q = self.in_src(p)?;
        self.ground.height(q[0], q[1])
    }

    /// Surface height (m) at a drawing point: roofs, trees, bridge decks
    /// (AHN DSM); elsewhere the ground.
    pub fn surface(&mut self, p: XY) -> Option<f64> {
        let q = self.in_src(p)?;
        match self.surface.as_mut() {
            Some(s) => s.height(q[0], q[1]),
            None => self.ground.height(q[0], q[1]),
        }
    }

    /// The ground as an AHN grid of `cell` m over an RD box (the Dutch 3D model).
    pub fn grid(&self, bbox_rd: [f64; 4], cell: f64, progress: &dyn Fn(String)) -> Result<ahn::Grid, String> {
        ahn::fetch(bbox_rd, Model::Dtm, cell, progress)
    }

    pub fn error(&self) -> Option<String> {
        self.ground.error()
    }

    /// Ground points every `spacing` m in the import area, in drawing
    /// coordinates and units (for a TIN without survey points).
    pub fn grid_points(&self, ctx: &Ctx, spacing: f64, progress: &dyn Fn(String)) -> Result<Vec<terrein::P3>, String> {
        let b = self.src.bbox(&self.site)?;
        let cells = ((b[2] - b[0]) / spacing) * ((b[3] - b[1]) / spacing);
        if cells > 200_000.0 {
            return Err(format!("{cells:.0} {} {spacing} m", crate::lang::tr("punten is te veel: kies een grotere puntafstand dan", "points is too many: choose a spacing larger than")));
        }
        let grid = self.src.fetch(b, spacing, progress)?;
        let to = crate::gis::vector::data_to_site(&self.site, &self.src.crs, &[])?;
        let rect = self.site.drawing_rect(self.f);
        let mut out = Vec::with_capacity(grid.w * grid.h);
        for row in 0..grid.h {
            for col in 0..grid.w {
                let Some(z) = grid.at(col, row) else { continue };
                let Some(q) = to(grid.centre(col, row)) else { continue };
                let p = self.site.to_drawing(q, self.f);
                if crate::gis::vector::inside(p, rect) {
                    out.push([p[0], p[1], z * ctx.f]);
                }
            }
        }
        if out.is_empty() {
            return Err(format!("{} {}", self.src.label(), crate::lang::tr("heeft geen hoogtes in het importgebied", "has no heights in the import area")));
        }
        Ok(out)
    }

    pub fn label(&self) -> String {
        if self.src.is_ahn() { "AHN4 maaiveld (DTM)".into() } else { self.src.label() }
    }

    pub fn attribution(&self) -> &'static str {
        self.src.attribution()
    }

    /// The height reference: NAP or (about) mean sea level.
    pub fn datum(&self) -> &'static str {
        self.src.datum()
    }
}

/// Heights for profiles and sections: the TIN in the drawing when there is
/// one, AHN otherwise.
pub enum Maaiveld {
    Tin(terrein::Tin),
    Bron(Hoogtebron),
}

impl Maaiveld {
    /// The TIN on the triangulation layers of the drawing, or AHN.
    pub fn find(ctx: &Ctx, doc: &ocs_plugin_api::host::codec::CadDocument) -> Result<Maaiveld, String> {
        match terrein::Tin::from_document(doc) {
            Some(t) => Ok(Maaiveld::Tin(t)),
            None => Ok(Maaiveld::Bron(hoogtebron(ctx)?)),
        }
    }

    /// Height in metres at a drawing point (the TIN stores drawing units).
    pub fn height(&mut self, p: XY, f: f64) -> Option<f64> {
        match self {
            Maaiveld::Tin(t) => t.height(p).map(|z| z / f),
            Maaiveld::Bron(b) => b.height(p),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Maaiveld::Tin(t) => format!("terrein (TIN, {} driehoeken)", t.triangles.len()),
            Maaiveld::Bron(b) => b.label().to_string(),
        }
    }

    pub fn note_source(&self) {
        if let Maaiveld::Bron(b) = self {
            bridge::note_source(b.attribution());
        }
    }

    pub fn error(&self) -> Option<String> {
        match self {
            Maaiveld::Tin(_) => None,
            Maaiveld::Bron(b) => b.error(),
        }
    }
}

/// What the drawing holds for the civil tools, for scripts and agents:
/// project location, axes, profiles, terrains, corridors.
pub fn info() -> Result<String, String> {
    use serde_json::json;
    let opt = settings();
    let ctx = Ctx::new(&opt)?;
    let doc = document(&["tin", "as", "lengteprofiel", "ontwerpprofiel", "corridor", "punten"], &[])?;
    let f = ctx.f;
    let h = |e: &EntityType| api::handle_text(e.common().handle);
    let mut axes = Vec::new();
    let mut frames = Vec::new();
    let mut designs = Vec::new();
    let mut corridors = Vec::new();
    for e in doc.model_space_entities() {
        if let Some(name) = data_value(e, "lengteprofiel") {
            frames.push(json!({"handle": h(e), "naam": name, "as": data_value(e, "as")}));
        } else if let Some(name) = data_value(e, "ontwerpprofiel") {
            designs.push(json!({"handle": h(e), "naam": name, "as": data_value(e, "as"), "pvi": data_value(e, "pvi")}));
        } else if data_value(e, "corridor").is_some() && matches!(e, EntityType::Polyline3D(_)) {
            corridors.push(json!({"handle": h(e), "naam": data_value(e, "corridor"), "sjabloon": data_value(e, "sjabloon")}));
        } else if let (Some(name), Some(al)) = (data_value(e, "as"), as_profiel::Alignment::from_entity(e)) {
            if e.common().layer.contains("-AM-AS-") && matches!(e, EntityType::LwPolyline(_)) && data_value(e, "verschuiving_m").is_none() {
                axes.push(json!({"handle": h(e), "naam": name, "lengte_m": al.length() / f, "beginstation": al.start}));
            }
        }
    }
    let surface = |c: char| {
        terrein::Tin::of_status(&doc, c).map(|t| {
            let (lo, hi) = t.range();
            json!({"driehoeken": t.triangles.len(), "punten": t.points.len(), "hoogte_min": lo / f, "hoogte_max": hi / f})
        })
    };
    let points = doc.model_space_entities().filter(|e| matches!(e, EntityType::Point(_)) && e.common().layer.contains("-OG-HOOGTEPUNT-")).count();
    api::note("projectlocatie", ctx.site.as_ref().map(|s| json!({"naam": s.label, "crs": s.crs, "x": s.origin_e, "y": s.origin_n, "breedte": s.width, "hoogte": s.height, "absoluut": s.absolute})).unwrap_or(serde_json::Value::Null));
    api::note("eenheid_per_m", f);
    api::note("assen", axes.clone());
    api::note("lengteprofielen", frames.clone());
    api::note("ontwerpprofielen", designs.clone());
    api::note("corridors", corridors.clone());
    api::note("terrein_bestaand", surface('B').unwrap_or(serde_json::Value::Null));
    api::note("terrein_ontwerp", surface('N').unwrap_or(serde_json::Value::Null));
    api::note("meetpunten", points);
    Ok(format!(
        "{} as(sen), {} lengteprofiel(en), {} ontwerpprofiel(en), {} corridor(s), {} meetpunten; terrein bestaand {}, ontwerp {}.",
        axes.len(),
        frames.len(),
        designs.len(),
        corridors.len(),
        points,
        if surface('B').is_some() { "ja" } else { "nee" },
        if surface('N').is_some() { "ja" } else { "nee" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stations_read_the_dutch_way() {
        assert_eq!(station(0.0), "0+000");
        assert_eq!(station(120.0), "0+120");
        assert_eq!(station(1045.5), "1+045,50");
        assert_eq!(station(999.999), "1+000");
        assert_eq!(station(-20.0), "-0+020");
        assert_eq!(nice_step(0.3), 0.5);
        assert_eq!(nice_step(7.0), 10.0);
    }
}
