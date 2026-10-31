//! Dutch base data drawn conform NLCS: BGT (PDOK), the cadastral map
//! (Kadaster BRK) and KLIC deliveries. Every object lands on its NLCS layer
//! (`STATUS-DISCIPLINE-HOOFDGROEP-OBJECT-ELEMENT`) and carries its source
//! attributes as object data (XDATA "OPENINFRA").

use std::collections::BTreeMap;
use std::path::Path;

use ocs_plugin_api::host::codec::EntityType;
use ocs_plugin_api::host::ExtendedDataRecord;

use crate::bridge::{self, Target};
use crate::emit;
use crate::gis::crs::XY;
use crate::gis::site::Site;
use crate::gis::vector::{self, Feature};
use crate::gis::{bgt, klic, ogcapi};
use crate::nlcs::{self, Element, LayerSpec, Status};

/// How NLCS objects are drawn.
#[derive(Clone, Debug)]
pub struct NlcsOptions {
    pub status: Status,
    pub discipline: String,
    /// Plot scale denominator (500 for 1:500): linetype scale and text sizes.
    pub scale: f64,
    /// Attach the source attributes as XDATA.
    pub object_data: bool,
    /// Texts: street names and house numbers, parcel numbers, cable labels.
    pub labels: bool,
    pub clip: bool,
}

impl Default for NlcsOptions {
    fn default() -> Self {
        Self { status: Status::B, discipline: "XX".into(), scale: 500.0, object_data: true, labels: true, clip: true }
    }
}

impl NlcsOptions {
    /// Paper millimetres → drawing units.
    fn paper(&self, mm: f64, unit_factor: f64) -> f64 {
        mm * self.scale / 1000.0 * unit_factor
    }
}

fn target() -> Result<Target, String> {
    bridge::target().ok_or_else(|| "geen tekening actief — klik in Open CAD Studio op een knop van het GIS-tabblad".into())
}

/// Collects entities per layer and writes them as one undo step.
struct Batch {
    entities: Vec<EntityType>,
    layers: BTreeMap<String, usize>,
    first_data: Option<ExtendedDataRecord>,
}

impl Batch {
    fn new() -> Self {
        Self { entities: Vec::new(), layers: BTreeMap::new(), first_data: None }
    }

    fn push(&mut self, spec: &LayerSpec, ents: Vec<EntityType>, data: Option<&ExtendedDataRecord>) {
        if ents.is_empty() {
            return;
        }
        if !self.layers.contains_key(&spec.name) {
            bridge::ensure_nlcs_layer(spec);
        }
        *self.layers.entry(spec.name.clone()).or_insert(0) += ents.len();
        if self.first_data.is_none() {
            self.first_data = data.cloned();
        }
        self.entities.extend(ents);
    }

    fn write(self, label: &str) -> Result<(usize, usize), String> {
        let n = self.entities.len();
        let handles = bridge::add_entities_handles(label, self.entities)?;
        if let (Some(h), Some(r)) = (handles.first(), self.first_data) {
            bridge::register_object_data(*h, r);
        }
        Ok((n, self.layers.len()))
    }
}

fn record<'a>(on: bool, pairs: impl IntoIterator<Item = (&'a str, String)>) -> Option<ExtendedDataRecord> {
    on.then(|| bridge::object_data(pairs))
}

// ── BGT ─────────────────────────────────────────────────────────────────────

/// Attributes not worth carrying into the drawing. `creation_date` (the
/// objectBeginTijd) stays: a BGT mutation of the object needs it.
fn bgt_skip(key: &str) -> bool {
    key.ends_with("_codespace")
        || key.ends_with("_leeg")
        || matches!(key, "version" | "lv_publicatiedatum" | "tijdstip_registratie" | "eind_registratie" | "termination_date" | "in_onderzoek")
}

fn prop_string(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Fetch the BGT collections and draw them.
pub fn bgt(site: &Site, collections: &[&str], opt: &NlcsOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let data = bgt::fetch(site, collections, progress)?;
    progress("BGT tekenen…".into());
    bgt_place(site, &data, opt)
}

pub fn bgt_place(site: &Site, data: &[(String, Vec<Feature>)], opt: &NlcsOptions) -> Result<String, String> {
    let t = target()?;
    let f = t.unit_factor;
    let to_site = vector::data_to_site(site, "EPSG:28992", &[])?;
    let ltscale = opt.scale / 1000.0 * f;
    let mut batch = Batch::new();
    let mut unmapped = 0usize;
    for (collection, features) in data {
        for feat in features {
            let mut pairs: Vec<(&str, String)> = vec![("bron", "BGT".into()), ("objecttype", collection.clone())];
            for (k, v) in &feat.props {
                if !bgt_skip(k) {
                    if let Some(s) = prop_string(v) {
                        pairs.push((k.as_str(), s));
                    }
                }
            }
            if bgt::is_label(collection) {
                if !opt.labels {
                    continue;
                }
                let (Some(obj), Some((text, angle))) = (nlcs::bgt_label_object(collection), bgt::label(collection, feat)) else { continue };
                let spec = nlcs::layer(obj, opt.status, &opt.discipline, Element::T);
                let mm = if collection == "openbareruimtelabel" { 2.5 } else { 1.8 };
                let (_, pts) = vector::feature_to_drawing(site, feat, &*to_site, f, opt.clip);
                pairs.push(("nlcs", obj.id.to_string()));
                let d = record(opt.object_data, pairs);
                let ents = pts.iter().map(|p| emit::text(*p, &text, opt.paper(mm, f), emit::readable(angle), &spec.name, d.as_ref())).collect();
                batch.push(&spec, ents, d.as_ref());
                continue;
            }
            let id = nlcs::bgt_lookup(collection, &feat.props).map(|(id, _)| id).or_else(|| nlcs::bgt_default(collection));
            let Some(obj) = id.and_then(nlcs::object) else {
                unmapped += 1;
                continue;
            };
            pairs.push(("nlcs", obj.id.to_string()));
            let d = record(opt.object_data, pairs);
            let (lines, points) = vector::feature_to_drawing(site, feat, &*to_site, f, opt.clip);
            if !lines.is_empty() {
                let spec = nlcs::layer(obj, opt.status, &opt.discipline, Element::G);
                batch.push(&spec, emit::polylines(&lines, &spec.name, ltscale, d.as_ref()), d.as_ref());
            }
            if !points.is_empty() {
                let spec = nlcs::layer(obj, opt.status, &opt.discipline, Element::S);
                batch.push(&spec, emit::markers(&points, &spec.name, opt.paper(0.5, f), d.as_ref()), d.as_ref());
            }
        }
    }
    let total: usize = data.iter().map(|(_, v)| v.len()).sum();
    let (n, layers) = batch.write("BGT (NLCS)")?;
    bridge::note_source("BGT (PDOK, CC0)");
    let mut msg = format!("BGT: {total} objecten → {n} elementen op {layers} NLCS-lagen.");
    if unmapped > 0 {
        msg.push_str(&format!(" {unmapped} zonder NLCS-object overgeslagen."));
    }
    Ok(msg)
}

// ── Kadastrale kaart ────────────────────────────────────────────────────────

pub const BRK_API: &str = "https://api.pdok.nl/kadaster/brk-kadastrale-kaart/ogc/v1";

pub struct Cadastre {
    pub boundaries: Vec<Feature>,
    pub parcels: Vec<Feature>,
}

pub fn cadastre_fetch(site: &Site, numbers: bool, progress: &dyn Fn(String)) -> Result<Cadastre, String> {
    let b = rd_bbox(site)?;
    let (boundaries, _) = ogcapi::fetch_items(BRK_API, "kadastralegrens", b, 28992, None, 50_000, progress)?;
    let parcels = if numbers { ogcapi::fetch_items(BRK_API, "perceel", b, 28992, None, 50_000, progress)?.0 } else { Vec::new() };
    Ok(Cadastre { boundaries, parcels })
}

pub fn rd_bbox(site: &Site) -> Result<[f64; 4], String> {
    if site.crs == "EPSG:28992" {
        Ok(site.bbox())
    } else {
        crate::gis::site::envelope(site.bbox(), &crate::gis::crs::Transformer::new(&site.crs, "EPSG:28992")?)
    }
}

pub fn cadastre(site: &Site, opt: &NlcsOptions, progress: &dyn Fn(String)) -> Result<String, String> {
    let data = cadastre_fetch(site, opt.labels, progress)?;
    cadastre_place(site, &data, opt)
}

pub fn cadastre_place(site: &Site, data: &Cadastre, opt: &NlcsOptions) -> Result<String, String> {
    let t = target()?;
    let f = t.unit_factor;
    let to_site = vector::data_to_site(site, "EPSG:28992", &[])?;
    let obj = nlcs::object_by_name("KG", "GRENS_PERCEEL").ok_or("NLCS-object KG GRENS_PERCEEL ontbreekt")?;
    let line_spec = nlcs::layer(obj, opt.status, &opt.discipline, Element::G);
    let text_spec = nlcs::layer(obj, opt.status, &opt.discipline, Element::T);
    let ltscale = opt.scale / 1000.0 * f;
    let mut batch = Batch::new();
    for g in &data.boundaries {
        let (lines, _) = vector::feature_to_drawing(site, g, &*to_site, f, opt.clip);
        let get = |k: &str| g.props.get(k).and_then(prop_string).unwrap_or_default();
        let d = data_pairs(opt.object_data, &[("bron", "BRK".into()), ("objecttype", "kadastralegrens".into()), ("lokaal_id", get("identificatie_lokaal_id")), ("kwaliteit", get("classificatie_kwaliteit_waarde")), ("perceel_links", get("perceel_links_identificatie_lokaal_id")), ("perceel_rechts", get("perceel_rechts_identificatie_lokaal_id"))]);
        batch.push(&line_spec, emit::polylines(&lines, &line_spec.name, ltscale, d.as_ref()), d.as_ref());
    }
    for p in &data.parcels {
        let get = |k: &str| p.props.get(k).and_then(prop_string).unwrap_or_default();
        let number = get("perceelnummer");
        if number.is_empty() {
            continue;
        }
        let Some(ring) = p.lines.iter().find(|(_, closed)| *closed).map(|(r, _)| r) else { continue };
        let Some(at) = vector::label_point(ring).and_then(|q| to_site(q)).map(|q| site.to_drawing(q, f)) else { continue };
        if opt.clip && !vector::inside(at, site.drawing_rect(f)) {
            continue;
        }
        let rot = p.props.get("perceelnummer_rotatie").and_then(|v| v.as_f64()).unwrap_or(0.0).to_radians();
        let aanduiding = format!("{} {} {}", get("akr_kadastrale_gemeente_code_waarde"), get("sectie"), number);
        let d = data_pairs(opt.object_data, &[("bron", "BRK".into()), ("objecttype", "perceel".into()), ("kadastrale_aanduiding", aanduiding), ("gemeente", get("kadastrale_gemeente_waarde")), ("grootte_m2", get("kadastrale_grootte_waarde")), ("lokaal_id", get("identificatie_lokaal_id"))]);
        batch.push(&text_spec, vec![emit::text(at, &number, opt.paper(1.8, f), emit::readable(rot), &text_spec.name, d.as_ref())], d.as_ref());
    }
    let (n, _) = batch.write("Kadastrale kaart (NLCS)")?;
    bridge::note_source("BRK kadastrale kaart (Kadaster, CC BY 4.0)");
    Ok(format!("Kadastrale kaart: {} grenzen en {} perceelnummers ({n} elementen) op {}.", data.boundaries.len(), data.parcels.len(), line_spec.name))
}

fn data_pairs(on: bool, pairs: &[(&str, String)]) -> Option<ExtendedDataRecord> {
    record(on, pairs.iter().map(|(k, v)| (*k, v.clone())))
}

// ── KLIC ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct KlicOptions {
    pub nlcs: NlcsOptions,
    /// IMKL theme colours instead of the NLCS colours.
    pub imkl_colors: bool,
    /// Themes to draw (empty: all).
    pub themes: Vec<String>,
}

impl Default for KlicOptions {
    fn default() -> Self {
        Self { nlcs: NlcsOptions { discipline: "OI".into(), clip: false, ..NlcsOptions::default() }, imkl_colors: false, themes: Vec::new() }
    }
}

/// The KLIC delivery read last (for KLIC in 3D, which runs in the Civiel
/// plugin's own process): kept in the config directory.
pub fn last_klic() -> Option<std::path::PathBuf> {
    let p = std::fs::read_to_string(crate::gis::servers::config_dir().join("laatste-klic.txt")).ok()?;
    let p = std::path::PathBuf::from(p.trim());
    p.exists().then_some(p)
}

pub fn klic(site: &Site, path: &Path, opt: &KlicOptions) -> Result<String, String> {
    let lev = klic::read(path)?;
    let dir = crate::gis::servers::config_dir();
    let _ = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join("laatste-klic.txt"), path.to_string_lossy().as_bytes()));
    klic_place(site, &lev, opt)
}

pub fn klic_place(site: &Site, lev: &klic::Levering, opt: &KlicOptions) -> Result<String, String> {
    let t = target()?;
    let f = t.unit_factor;
    let o = &opt.nlcs;
    let to_site = vector::data_to_site(site, "EPSG:28992", &[])?;
    let ltscale = o.scale / 1000.0 * f;
    let mut batch = Batch::new();
    let mut drawn = 0usize;
    let mut placed: Vec<(String, XY)> = Vec::new();
    for feat in &lev.features {
        let thema = klic::thema(feat);
        if !opt.themes.is_empty() && !opt.themes.iter().any(|t| t == thema) {
            continue;
        }
        let kind = klic::kind(feat);
        let obj = if thema == "graafpolygoon" { nlcs::object_by_name("ZZ", "GRENS_WERK") } else { nlcs::klic_object(thema, kind) };
        let Some(obj) = obj else { continue };
        let paint = |mut spec: LayerSpec| {
            if opt.imkl_colors && thema != "graafpolygoon" {
                spec.true_color = Some(nlcs::imkl_rgb(thema));
            }
            spec
        };
        let mut pairs: Vec<(String, String)> = vec![("bron".into(), "KLIC".into()), ("meldnummer".into(), lev.meldnummer.clone())];
        pairs.extend(klic::attributes(feat));
        pairs.push(("nlcs".into(), obj.id.to_string()));
        let d = record(o.object_data, pairs.iter().map(|(k, v)| (k.as_str(), v.clone())));
        let (lines, points) = vector::feature_to_drawing(site, feat, &*to_site, f, o.clip);
        if lines.is_empty() && points.is_empty() {
            continue;
        }
        drawn += 1;
        if !lines.is_empty() {
            let spec = paint(nlcs::layer(obj, o.status, &o.discipline, Element::G));
            batch.push(&spec, emit::polylines(&lines, &spec.name, ltscale, d.as_ref()), d.as_ref());
            let casing = matches!(kind, "Mantelbuis" | "Kabelbed");
            if o.labels && !casing && thema != "graafpolygoon" {
                let text = klic::label(feat);
                if let Some((at, angle)) = lines.iter().max_by(|a, b| length(&a.0).total_cmp(&length(&b.0))).and_then(|(l, _)| emit::label_spot(l)) {
                    if !text.is_empty() && length(&lines[0].0) > o.paper(12.0, f) {
                        let spec = paint(nlcs::layer(obj, o.status, &o.discipline, Element::T));
                        let h = o.paper(1.8, f);
                        // Beside the line, not on it; one label for parallel
                        // cables of the same operator.
                        let off = [-angle.sin() * h * 0.9, angle.cos() * h * 0.9];
                        let spot = [at[0] + off[0], at[1] + off[1]];
                        let near = o.paper(20.0, f);
                        let duplicate = placed.iter().any(|(t, p): &(String, XY)| *t == text && (p[0] - spot[0]).hypot(p[1] - spot[1]) < near);
                        if !duplicate {
                            placed.push((text.clone(), spot));
                            let ents = vec![emit::text(spot, &text, h, emit::readable(angle), &spec.name, d.as_ref())];
                            batch.push(&spec, ents, d.as_ref());
                        }
                    }
                }
            }
        }
        if !points.is_empty() {
            let spec = paint(nlcs::layer(obj, o.status, &o.discipline, Element::S));
            batch.push(&spec, emit::markers(&points, &spec.name, o.paper(0.6, f), d.as_ref()), d.as_ref());
        }
    }
    let (n, layers) = batch.write(&format!("KLIC {}", lev.meldnummer))?;
    bridge::note_source(&format!("KLIC-levering {} (Kadaster)", lev.meldnummer));
    Ok(format!("KLIC {}: {drawn} objecten → {n} elementen op {layers} NLCS-lagen ({} netbeheerders).", lev.meldnummer, lev.operators.len()))
}

fn length(pts: &[XY]) -> f64 {
    pts.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum()
}
