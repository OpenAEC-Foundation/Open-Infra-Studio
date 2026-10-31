//! BRO (Basisregistratie Ondergrond, the successor of DINOloket): cone
//! penetration tests (CPT) and geotechnical borehole descriptions (BHR-GT)
//! from the public REST services at publiek.broservices.nl (open data, CC0).
//!
//! A search (`POST …/characteristics/searches` with a WGS 84 box) lists the
//! objects of an area with their location, surface level and end depth; the
//! object (`GET …/objects/{broId}`) is IMBRO XML with the measurements. For
//! CPTs the soil type per depth follows from cone resistance and friction
//! ratio ([`classify`], Robertson 2010); boreholes carry their described
//! soil names (NEN-EN-ISO 14688 or NEN 5104), mapped by [`soil_of_name`].

use roxmltree::{Document, Node};

use super::crs::XY;

pub const CPT_API: &str = "https://publiek.broservices.nl/sr/cpt/v1";
pub const BHR_API: &str = "https://publiek.broservices.nl/sr/bhrgt/v2";
/// The services refuse a search with more objects than this.
pub const SEARCH_LIMIT: usize = 2000;
/// Missing value in the CPT result block.
const VOID: f64 = -999_999.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Sondering (cone penetration test).
    Cpt,
    /// Geotechnisch booronderzoek (borehole with soil description).
    Bhr,
}

impl Kind {
    fn api(self) -> &'static str {
        match self {
            Kind::Cpt => CPT_API,
            Kind::Bhr => BHR_API,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Cpt => "sondering",
            Kind::Bhr => "boring",
        }
    }
}

/// One BRO object as the search lists it (and the head of the object itself).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Point {
    pub id: String,
    pub cpt: bool,
    /// Delivered location in RD (EPSG:28992), when delivered in RD.
    pub rd: Option<XY>,
    /// Standardized location (ETRS89 ≈ WGS 84): lon, lat.
    pub lonlat: Option<XY>,
    /// Height of the reference point (normally the ground level) in `datum`.
    pub surface: Option<f64>,
    /// Vertical datum, normally NAP.
    pub datum: String,
    /// Reference point: maaiveld, waterbodem, …
    pub reference: String,
    /// End depth below the reference point (m).
    pub end_depth: Option<f64>,
    /// Date of the test or boring (yyyy-mm-dd).
    pub date: String,
    /// IMBRO or IMBRO/A (migrated archive data).
    pub regime: String,
    pub deregistered: bool,
}

impl Point {
    pub fn kind(&self) -> Kind {
        if self.cpt { Kind::Cpt } else { Kind::Bhr }
    }
}

// ── XML helpers (local names: the BRO mixes several namespaces) ─────────────

fn first<'a, 'i>(n: Node<'a, 'i>, local: &str) -> Option<Node<'a, 'i>> {
    n.descendants().find(|d| d.is_element() && d.tag_name().name() == local)
}

fn text_of(n: Node, local: &str) -> Option<String> {
    first(n, local).and_then(|e| e.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn num_of(n: Node, local: &str) -> Option<f64> {
    text_of(n, local).and_then(|t| t.parse().ok())
}

fn pos(n: Node) -> Option<(XY, String)> {
    let p = first(n, "pos")?;
    let v: Vec<f64> = p.text()?.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    let srs = n.descendants().chain(p.ancestors()).find_map(|e| e.attribute("srsName")).unwrap_or("").to_string();
    (v.len() >= 2).then(|| ([v[0], v[1]], srs))
}

/// The rejection of a request (`brocom:responseType` "rejection").
fn rejection(doc: &Document) -> Option<String> {
    let root = doc.root_element();
    (text_of(root, "responseType").as_deref() == Some("rejection")).then(|| text_of(root, "rejectionReason").unwrap_or_else(|| "verzoek geweigerd".into()))
}

/// The head of a search document or object: location, surface, depth, date.
fn point_of(n: Node, cpt: bool) -> Point {
    let mut p = Point { cpt, id: text_of(n, "broId").unwrap_or_default(), ..Default::default() };
    p.deregistered = text_of(n, "deregistered").as_deref() == Some("ja");
    if let Some((xy, srs)) = first(n, "deliveredLocation").and_then(pos) {
        if srs.contains("28992") {
            p.rd = Some(xy);
        }
    }
    // EPSG:4258 lists latitude first.
    p.lonlat = first(n, "standardizedLocation").and_then(pos).map(|(xy, _)| [xy[1], xy[0]]);
    p.surface = num_of(n, "offset");
    p.datum = text_of(n, "verticalDatum").unwrap_or_default();
    p.reference = text_of(n, "localVerticalReferencePoint").unwrap_or_default();
    p.end_depth = num_of(n, if cpt { "finalDepth" } else { "finalDepthBoring" });
    let date = if cpt {
        text_of(n, "startTime").or_else(|| text_of(n, "timePosition"))
    } else {
        first(n, "boringStartDate").and_then(|d| text_of(d, "date"))
    };
    p.date = date.map(|d| d.chars().take(10).collect()).unwrap_or_default();
    p.regime = text_of(n, "qualityRegime").unwrap_or_default();
    p
}

/// The objects of a characteristics search (deregistered ones left out).
pub fn parse_characteristics(xml: &str) -> Result<Vec<Point>, String> {
    let doc = Document::parse(xml).map_err(|e| format!("BRO: onleesbaar antwoord ({e})"))?;
    if let Some(r) = rejection(&doc) {
        return Err(format!("BRO: {r}"));
    }
    Ok(doc
        .root_element()
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "dispatchDocument")
        .filter_map(|d| d.children().find(|c| c.is_element()))
        .map(|o| point_of(o, o.tag_name().name().starts_with("CPT")))
        .filter(|p| !p.id.is_empty() && !p.deregistered)
        .collect())
}

// ── Search ──────────────────────────────────────────────────────────────────

/// All objects of `kind` in a WGS 84 box [w, s, e, n]. A box with more than
/// the services' limit is split in four, up to four levels deep.
pub fn search(kind: Kind, lonlat: [f64; 4]) -> Result<Vec<Point>, String> {
    let mut out = search_box(kind, lonlat, 0)?;
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(p.id.clone()));
    Ok(out)
}

fn search_box(kind: Kind, b: [f64; 4], depth: u32) -> Result<Vec<Point>, String> {
    let body = serde_json::json!({"area": {"boundingBox": {"lowerCorner": {"lat": b[1], "lon": b[0]}, "upperCorner": {"lat": b[3], "lon": b[2]}}}});
    let url = format!("{}/characteristics/searches", kind.api());
    let r = super::http::post_json(&url, &body, 120).and_then(|f| String::from_utf8(f.body).map_err(|_| "BRO: geen UTF-8".to_string())).and_then(|t| parse_characteristics(&t));
    match r {
        Err(e) if e.contains(&SEARCH_LIMIT.to_string()) && depth < 4 => {
            let (mx, my) = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
            let mut all = Vec::new();
            for q in [[b[0], b[1], mx, my], [mx, b[1], b[2], my], [b[0], my, mx, b[3]], [mx, my, b[2], b[3]]] {
                all.extend(search_box(kind, q, depth + 1)?);
            }
            Ok(all)
        }
        r => r.map_err(|e| format!("BRO {}: {e}", kind.label())),
    }
}

fn object_xml(kind: Kind, id: &str) -> Result<String, String> {
    super::http::get_text(&format!("{}/objects/{}", kind.api(), super::http::encode(id))).map_err(|e| format!("BRO {id}: {e}"))
}

// ── CPT ─────────────────────────────────────────────────────────────────────

/// The columns of the CPT result block (`cptcommon:values`), in the order of
/// `cptcommon:parameters` and of the IMBRO catalogue (25 columns).
pub const CPT_COLUMNS: [&str; 25] = [
    "penetrationLength",
    "depth",
    "elapsedTime",
    "coneResistance",
    "correctedConeResistance",
    "netConeResistance",
    "magneticFieldStrengthX",
    "magneticFieldStrengthY",
    "magneticFieldStrengthZ",
    "magneticFieldStrengthTotal",
    "electricalConductivity",
    "inclinationEW",
    "inclinationNS",
    "inclinationX",
    "inclinationY",
    "inclinationResultant",
    "magneticInclination",
    "magneticDeclination",
    "localFriction",
    "poreRatio",
    "temperature",
    "porePressureU1",
    "porePressureU2",
    "porePressureU3",
    "frictionRatio",
];

/// One CPT measurement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Depth below the reference point (m): the inclination-corrected depth
    /// when measured, else the penetration length.
    pub depth: f64,
    /// Cone resistance qc (MPa).
    pub qc: f64,
    /// Local friction fs (MPa).
    pub fs: Option<f64>,
    /// Friction ratio Rf = fs / qc (%).
    pub rf: Option<f64>,
}

#[derive(Clone, Debug, Default)]
pub struct Cpt {
    pub point: Point,
    /// By depth, without repeated depths (rod changes) and missing qc.
    pub samples: Vec<Sample>,
    pub predrilled: f64,
}

pub fn parse_cpt(xml: &str) -> Result<Cpt, String> {
    let doc = Document::parse(xml).map_err(|e| format!("BRO: onleesbare sondering ({e})"))?;
    if let Some(r) = rejection(&doc) {
        return Err(format!("BRO: {r}"));
    }
    let root = doc.root_element();
    let obj = first(root, "CPT_O").ok_or("BRO: geen sondering (CPT_O) in het antwoord")?;
    let point = point_of(obj, true);
    let columns: Vec<String> = match first(obj, "parameters") {
        Some(p) => p.children().filter(|c| c.is_element()).map(|c| c.tag_name().name().to_string()).collect(),
        None => CPT_COLUMNS.iter().map(|c| c.to_string()).collect(),
    };
    let col = |name: &str| columns.iter().position(|c| c == name);
    let (pl, dp, qc, fs, rf) = (col("penetrationLength"), col("depth"), col("coneResistance"), col("localFriction"), col("frictionRatio"));
    let qc = qc.ok_or("BRO: sondering zonder conusweerstand")?;
    let enc = first(obj, "TextEncoding");
    let sep = |a: &str, d: &str| enc.and_then(|e| e.attribute(a)).unwrap_or(d).to_string();
    let (dec, tok, block) = (sep("decimalSeparator", "."), sep("tokenSeparator", ","), sep("blockSeparator", ";"));
    let values = obj
        .descendants()
        .find(|d| d.is_element() && d.tag_name().name() == "values" && d.parent().is_some_and(|p| p.tag_name().name() == "cptResult"))
        .and_then(|v| v.text())
        .unwrap_or("");
    let mut rows: Vec<(f64, Sample)> = Vec::new();
    for r in values.split(block.as_str()).filter(|r| !r.trim().is_empty()) {
        let v: Vec<f64> = r.split(tok.as_str()).map(|t| if dec == "." { t.trim().parse() } else { t.trim().replace(dec.as_str(), ".").parse() }.unwrap_or(VOID)).collect();
        let get = |i: Option<usize>| i.and_then(|i| v.get(i).copied()).filter(|x| *x > VOID + 1.0);
        let length = get(pl).or(get(dp));
        let (Some(length), Some(q)) = (length, get(Some(qc))) else { continue };
        if q <= 0.0 {
            continue;
        }
        let f = get(fs).filter(|f| *f >= 0.0);
        let ratio = get(rf).filter(|r| *r >= 0.0).or_else(|| f.map(|f| f / q * 100.0));
        rows.push((length, Sample { depth: get(dp).unwrap_or(length), qc: q, fs: f, rf: ratio }));
    }
    // The block may hold the rod changes twice and be out of order.
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut samples: Vec<Sample> = Vec::with_capacity(rows.len());
    for (_, s) in rows {
        if samples.last().is_none_or(|l| s.depth > l.depth + 1e-6) {
            samples.push(s);
        }
    }
    let predrilled = num_of(obj, "predrilledDepth").unwrap_or(0.0);
    Ok(Cpt { point, samples, predrilled })
}

pub fn fetch_cpt(id: &str) -> Result<Cpt, String> {
    parse_cpt(&object_xml(Kind::Cpt, id)?)
}

// ── Soil types ──────────────────────────────────────────────────────────────

/// Soil classes for the columns, in conventional colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Grond {
    Grind,
    Zand,
    SiltigZand,
    /// Silt and leem (loam).
    Silt,
    Klei,
    Veen,
    /// Made ground and other material (puin, verharding, beton…).
    Antropogeen,
    /// No measurement or description.
    Onbekend,
}

impl Grond {
    pub const ALL: [Grond; 8] = [Grond::Grind, Grond::Zand, Grond::SiltigZand, Grond::Silt, Grond::Klei, Grond::Veen, Grond::Antropogeen, Grond::Onbekend];

    pub fn label(self) -> &'static str {
        match self {
            Grond::Grind => "grind",
            Grond::Zand => "zand",
            Grond::SiltigZand => "siltig/kleiig zand",
            Grond::Silt => "silt/leem",
            Grond::Klei => "klei",
            Grond::Veen => "veen",
            Grond::Antropogeen => "antropogeen",
            Grond::Onbekend => "onbekend",
        }
    }

    /// Conventional colours of Dutch borehole logs: sand yellow, clay green,
    /// peat brown, silt/loam grey-green, gravel orange.
    pub fn rgb(self) -> (u8, u8, u8) {
        match self {
            Grond::Grind => (230, 146, 52),
            Grond::Zand => (246, 214, 72),
            Grond::SiltigZand => (214, 206, 118),
            Grond::Silt => (172, 196, 150),
            Grond::Klei => (86, 160, 82),
            Grond::Veen => (128, 84, 46),
            Grond::Antropogeen => (150, 150, 156),
            Grond::Onbekend => (210, 210, 210),
        }
    }
}

/// Robertson's (2010) soil behaviour type index from the non-normalised
/// chart: Isbt = √((3.47 − log₁₀(qc/pa))² + (log₁₀ Rf + 1.22)²), with qc in
/// MPa, pa = 0.1 MPa (atmospheric pressure) and Rf in %. (P.K. Robertson,
/// "Soil behaviour type from the CPT: an update", CPT'10, Huntington Beach.)
pub fn isbt(qc: f64, rf: f64) -> f64 {
    let q = (qc.max(0.01) / 0.1).log10();
    let r = rf.max(0.05).log10();
    ((3.47 - q).powi(2) + (r + 1.22).powi(2)).sqrt()
}

/// The soil behaviour type zone of Robertson (2010) from [`isbt`]:
/// 2 organic soil/peat (> 3.60), 3 clay (2.95–3.60), 4 silt mixtures
/// (2.60–2.95), 5 sand mixtures (2.05–2.60), 6 sand (1.31–2.05),
/// 7 gravelly to dense sand (< 1.31). Zones 1, 8 and 9 (sensitive and very
/// stiff soils) need the normalised chart and are not used.
pub fn sbt_zone(qc: f64, rf: f64) -> u8 {
    match isbt(qc, rf) {
        i if i > 3.60 => 2,
        i if i > 2.95 => 3,
        i if i > 2.60 => 4,
        i if i > 2.05 => 5,
        i if i > 1.31 => 6,
        _ => 7,
    }
}

/// A CPT sample's soil class (Robertson 2010 SBT zone) and the zone's name.
pub fn classify(qc: f64, rf: f64) -> (Grond, &'static str) {
    match sbt_zone(qc, rf) {
        2 => (Grond::Veen, "veen/organische grond (SBT 2)"),
        3 => (Grond::Klei, "klei (SBT 3)"),
        4 => (Grond::Silt, "siltige klei tot kleiige silt (SBT 4)"),
        5 => (Grond::SiltigZand, "siltig zand tot zandige silt (SBT 5)"),
        6 => (Grond::Zand, "zand tot siltig zand (SBT 6)"),
        _ => (Grond::Grind, "grindig tot vast zand (SBT 7)"),
    }
}

/// A soil layer: depths below the reference point (m).
#[derive(Clone, Debug, PartialEq)]
pub struct Interval {
    pub top: f64,
    pub bottom: f64,
    pub soil: Grond,
    /// The description: the SBT zone name, or the borehole's soil name.
    pub name: String,
    /// Mean cone resistance (MPa) and friction ratio (%) of a CPT layer.
    pub qc: Option<f64>,
    pub rf: Option<f64>,
}

impl Interval {
    pub fn thickness(&self) -> f64 {
        self.bottom - self.top
    }
}

/// A gap between samples larger than this is a layer without measurement.
const GAP: f64 = 0.5;

/// Soil layers of a CPT: each sample classified, consecutive samples of the
/// same class merged, then layers thinner than `min` (m) merged into their
/// thicker neighbour until none is left. Each sample stands for the depths
/// halfway to its neighbours; gaps over 0.5 m (and a predrilled top) become
/// layers "onbekend" that are never merged. Samples without a friction
/// ratio count as missing.
pub fn cpt_intervals(samples: &[Sample], min: f64) -> Vec<Interval> {
    let samples: Vec<Sample> = samples.iter().filter(|s| s.rf.is_some()).copied().collect();
    let mut out: Vec<Interval> = Vec::new();
    let mut start = 0;
    let mut last_bottom = 0.0;
    while start < samples.len() {
        let mut end = start + 1;
        while end < samples.len() && samples[end].depth - samples[end - 1].depth <= GAP {
            end += 1;
        }
        let run = &samples[start..end];
        let top = run[0].depth;
        if top - last_bottom > 0.05 {
            out.push(Interval { top: last_bottom, bottom: top, soil: Grond::Onbekend, name: "geen meting".into(), qc: None, rf: None });
        }
        let bottom = run[run.len() - 1].depth;
        out.extend(run_intervals(run, min));
        last_bottom = bottom;
        start = end;
    }
    out
}

/// One continuous run of samples → layers.
fn run_intervals(run: &[Sample], min: f64) -> Vec<Interval> {
    // (top, bottom, class, name, Σqc·h, Σrf·h, Σh of rf)
    struct L {
        top: f64,
        bottom: f64,
        soil: Grond,
        name: &'static str,
        qh: f64,
        rh: f64,
        rw: f64,
    }
    let n = run.len();
    let mut ls: Vec<L> = Vec::new();
    for (i, s) in run.iter().enumerate() {
        let top = if i == 0 { s.depth } else { (run[i - 1].depth + s.depth) / 2.0 };
        let bottom = if i + 1 == n { s.depth } else { (s.depth + run[i + 1].depth) / 2.0 };
        let (soil, name) = match s.rf {
            Some(rf) => classify(s.qc, rf),
            None => (Grond::Onbekend, "geen kleefmeting"),
        };
        let h = bottom - top;
        let (rh, rw) = s.rf.map(|r| (r * h, h)).unwrap_or((0.0, 0.0));
        match ls.last_mut() {
            Some(l) if l.soil == soil => {
                l.bottom = bottom;
                l.qh += s.qc * h;
                l.rh += rh;
                l.rw += rw;
            }
            _ => ls.push(L { top, bottom, soil, name, qh: s.qc * h, rh, rw }),
        }
    }
    // A single sample has no thickness: keep it as a thin layer.
    while ls.len() > 1 {
        let Some((i, _)) = ls.iter().enumerate().filter(|(_, l)| l.bottom - l.top < min).min_by(|a, b| (a.1.bottom - a.1.top).total_cmp(&(b.1.bottom - b.1.top))) else { break };
        let th = |l: &L| l.bottom - l.top;
        let into = if i == 0 {
            1
        } else if i + 1 == ls.len() || th(&ls[i - 1]) >= th(&ls[i + 1]) {
            i - 1
        } else {
            i + 1
        };
        let l = ls.remove(i);
        let j = if into > i { into - 1 } else { into };
        let t = &mut ls[j];
        t.top = t.top.min(l.top);
        t.bottom = t.bottom.max(l.bottom);
        t.qh += l.qh;
        t.rh += l.rh;
        t.rw += l.rw;
        // Neighbours of the same class join.
        let mut k = 1;
        while k < ls.len() {
            if ls[k].soil == ls[k - 1].soil {
                let b = ls.remove(k);
                let a = &mut ls[k - 1];
                a.bottom = b.bottom;
                a.qh += b.qh;
                a.rh += b.rh;
                a.rw += b.rw;
            } else {
                k += 1;
            }
        }
    }
    ls.into_iter()
        .map(|l| {
            let h = l.bottom - l.top;
            Interval {
                top: l.top,
                bottom: l.bottom,
                soil: l.soil,
                name: l.name.to_string(),
                qc: (h > 0.0).then(|| l.qh / h),
                rf: (l.rw > 0.0).then(|| l.rh / l.rw),
            }
        })
        .collect()
}

/// "zwakSiltigZand" → "zwak siltig zand".
pub fn humanize(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push(' ');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// The class of a described soil name, NEN-EN-ISO 14688 (geotechnicalSoilName,
/// e.g. "kleiigZand", "zwakZandigeKleiMetGrind") or NEN 5104 (soilNameNEN5104,
/// e.g. "matigSiltigeKlei", "mineraalarmVeen"): the main noun decides; sand
/// with a strong or moderate (or ISO's ungraded) silt or clay admixture is
/// silty sand; "…MetGrind" is an admixture, not the main soil.
pub fn soil_of_name(name: &str) -> Grond {
    let main = match name.find("Met").filter(|i| *i > 0) {
        Some(i) => &name[..i],
        None => name,
    };
    let m = main.to_ascii_lowercase();
    if m.ends_with("veen") {
        Grond::Veen
    } else if m.ends_with("klei") {
        Grond::Klei
    } else if m.ends_with("leem") || m.ends_with("silt") {
        Grond::Silt
    } else if m.ends_with("zand") {
        let mixed = m.contains("siltig") || m.contains("kleiig");
        if mixed && !m.starts_with("zwak") { Grond::SiltigZand } else { Grond::Zand }
    } else if m.ends_with("grind") || m.ends_with("keien") || m.ends_with("blokken") || m.ends_with("stenen") {
        Grond::Grind
    } else {
        Grond::Onbekend
    }
}

// ── BHR-GT ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct Bhr {
    pub point: Point,
    /// The described layers, from the log that covers most depth.
    pub layers: Vec<Interval>,
    /// The log's description procedure (ISO14688…, NEN5104…).
    pub procedure: String,
}

fn layer_of(l: Node) -> Option<Interval> {
    let top = num_of(l, "upperBoundary")?;
    let bottom = num_of(l, "lowerBoundary")?;
    let soil_name = text_of(l, "geotechnicalSoilName").or_else(|| text_of(l, "soilNameNEN5104"));
    let (soil, name) = match soil_name {
        Some(n) => (soil_of_name(&n), humanize(&n)),
        None => match text_of(l, "specialMaterial") {
            Some(m) => (Grond::Antropogeen, humanize(&m)),
            None => (Grond::Onbekend, "niet beschreven".into()),
        },
    };
    let name = if text_of(l, "anthropogenic").as_deref() == Some("ja") && soil != Grond::Antropogeen { format!("{name} (opgebracht)") } else { name };
    (bottom > top).then_some(Interval { top, bottom, soil, name, qc: None, rf: None })
}

pub fn parse_bhr(xml: &str) -> Result<Bhr, String> {
    let doc = Document::parse(xml).map_err(|e| format!("BRO: onleesbare boring ({e})"))?;
    if let Some(r) = rejection(&doc) {
        return Err(format!("BRO: {r}"));
    }
    let obj = first(doc.root_element(), "BHR_GT_O").ok_or("BRO: geen boring (BHR_GT_O) in het antwoord")?;
    let point = point_of(obj, false);
    let desc = first(obj, "boreholeSampleDescription");
    // A field log covers the whole boring; a lab log only its samples.
    let layers = desc
        .into_iter()
        .flat_map(|d| d.descendants().filter(|n| n.is_element() && n.tag_name().name() == "descriptiveBoreholeLog"))
        .map(|log| log.children().filter(|c| c.is_element() && c.tag_name().name() == "layer").filter_map(layer_of).collect::<Vec<_>>())
        .max_by(|a, b| a.iter().map(Interval::thickness).sum::<f64>().total_cmp(&b.iter().map(Interval::thickness).sum::<f64>()))
        .unwrap_or_default();
    let procedure = desc.and_then(|d| text_of(d, "descriptionProcedure")).unwrap_or_default();
    Ok(Bhr { point, layers, procedure })
}

pub fn fetch_bhr(id: &str) -> Result<Bhr, String> {
    parse_bhr(&object_xml(Kind::Bhr, id)?)
}

/// Fetch objects on a few threads at once; failures are returned per id.
pub fn fetch_many<T: Send>(ids: &[String], f: fn(&str) -> Result<T, String>, progress: &dyn Fn(String)) -> Vec<Result<T, String>> {
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<Result<T, String>>>> = ids.iter().map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..6.min(ids.len()) {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(id) = ids.get(i) else { break };
                *slots[i].lock().unwrap() = Some(f(id));
                done.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            });
        }
        // Progress from this thread only (the callback need not be Sync).
        loop {
            let d = done.load(std::sync::atomic::Ordering::SeqCst);
            progress(format!("BRO: {d}/{} objecten…", ids.len()));
            if d >= ids.len() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    });
    slots.into_iter().map(|m| m.into_inner().unwrap().unwrap_or_else(|| Err("niet opgehaald".into()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/bro/");

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{DIR}{name}")).unwrap()
    }

    #[test]
    fn characteristics_of_a_search() {
        let pts = parse_characteristics(&fixture("cpt-characteristics.xml")).unwrap();
        assert_eq!(pts.len(), 3);
        let p = &pts[0];
        assert_eq!(p.id, "CPT000000011051");
        assert!(p.cpt);
        assert_eq!(p.rd, Some([104603.0, 424959.0]));
        let ll = p.lonlat.unwrap();
        assert!((ll[0] - 4.656393150).abs() < 1e-9 && (ll[1] - 51.810978890).abs() < 1e-9, "{ll:?}");
        assert_eq!((p.surface, p.end_depth), (Some(2.86), Some(26.18)));
        assert_eq!((p.datum.as_str(), p.reference.as_str(), p.date.as_str(), p.regime.as_str()), ("NAP", "maaiveld", "1997-04-24", "IMBRO/A"));
        let b = parse_characteristics(&fixture("bhrgt-characteristics.xml")).unwrap();
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].id.as_str(), b[0].cpt, b[0].rd, b[0].surface, b[0].end_depth), ("BHR000000360509", false, Some([104080.16, 424379.25]), Some(0.27), Some(11.6)));
        assert_eq!(b[0].date, "2020-11-05");
        // More than 2000 objects: the service's reason, for a split search.
        let e = parse_characteristics(&fixture("cpt-rejection.xml")).unwrap_err();
        assert!(e.contains("groter dan 2000"), "{e}");
        assert!(super::super::http::service_error(&fixture("cpt-rejection.xml")).unwrap().contains("2000"));
    }

    #[test]
    fn a_cpt_object() {
        let c = parse_cpt(&fixture("cpt-CPT000000091945.xml")).unwrap();
        assert_eq!(c.point.id, "CPT000000091945");
        assert_eq!(c.point.rd, Some([104695.84, 425221.2]));
        assert_eq!((c.point.surface, c.point.end_depth, c.point.date.as_str()), (Some(2.47), Some(25.225), "2018-10-02"));
        // The block: 0.00 (no qc), 0.02 … 2.38 and 6.72 … 7.30 with a rod
        // change written twice; sorted, without repeats.
        let s = &c.samples;
        assert!(s.windows(2).all(|w| w[1].depth > w[0].depth));
        assert_eq!(s[0], Sample { depth: 0.02, qc: 1.414, fs: Some(0.005), rf: Some(0.1) });
        let at = |d: f64| s.iter().find(|x| (x.depth - d).abs() < 0.0015).unwrap();
        assert_eq!((at(1.258).qc, at(1.258).fs, at(1.258).rf), (8.411, Some(0.046), Some(0.5)));
        assert!(s.iter().any(|x| x.depth > 7.0));
        let n = s.len();
        assert!((140..=152).contains(&n), "{n}");
        // Layers: sand on top, a gap without measurement, then deeper layers.
        let iv = cpt_intervals(s, 0.2);
        assert_eq!(iv[0].top, 0.02);
        assert!(iv.iter().any(|i| i.soil == Grond::Onbekend && i.thickness() > 3.0), "{iv:#?}");
        assert!(iv.windows(2).all(|w| (w[1].top - w[0].bottom).abs() < 1e-9 && w[0].soil != w[1].soil), "{iv:#?}");
        assert!(iv.iter().filter(|i| i.soil != Grond::Onbekend).all(|i| i.thickness() >= 0.2 - 1e-9), "{iv:#?}");
        assert!(matches!(iv[0].soil, Grond::Zand | Grond::Grind | Grond::SiltigZand), "{iv:#?}");
    }

    #[test]
    fn robertson_zones() {
        // Typical Dutch soils: Holocene peat, soft clay, silty sand, sand,
        // dense Pleistocene sand.
        assert_eq!(classify(0.3, 8.0).0, Grond::Veen);
        assert_eq!(classify(0.5, 4.0).0, Grond::Klei);
        assert_eq!(classify(1.5, 2.0).0, Grond::Silt);
        assert_eq!(classify(4.0, 1.2).0, Grond::SiltigZand);
        assert_eq!(classify(10.0, 0.6).0, Grond::Zand);
        assert_eq!(classify(30.0, 0.3).0, Grond::Grind);
        assert!((isbt(10.0, 0.6) - 1.78).abs() < 0.01, "{}", isbt(10.0, 0.6));
    }

    #[test]
    fn layers_merge_and_thin_ones_go() {
        let s = |d: f64, qc: f64, rf: f64| Sample { depth: d, qc, fs: None, rf: Some(rf) };
        // 1 m sand, one clay sample (0.1 m), 1 m sand, 1 m peat.
        let mut v = Vec::new();
        for i in 0..=10 {
            v.push(s(i as f64 * 0.1, 10.0, 0.6));
        }
        v.push(s(1.1, 0.5, 4.0));
        for i in 12..=21 {
            v.push(s(i as f64 * 0.1, 10.0, 0.6));
        }
        for i in 22..=31 {
            v.push(s(i as f64 * 0.1, 0.3, 8.0));
        }
        let iv = cpt_intervals(&v, 0.2);
        assert_eq!(iv.iter().map(|i| i.soil).collect::<Vec<_>>(), vec![Grond::Zand, Grond::Veen], "{iv:#?}");
        assert!((iv[0].bottom - 2.15).abs() < 1e-9 && (iv[1].bottom - 3.1).abs() < 1e-9, "{iv:#?}");
        assert!((iv[1].qc.unwrap() - 0.3).abs() < 1e-9 && (iv[1].rf.unwrap() - 8.0).abs() < 1e-9);
        // A gap of a metre is a layer without measurement.
        let g = cpt_intervals(&[s(0.0, 10.0, 0.6), s(0.2, 10.0, 0.6), s(1.5, 10.0, 0.6), s(1.7, 10.0, 0.6)], 0.2);
        assert_eq!(g.iter().map(|i| i.soil).collect::<Vec<_>>(), vec![Grond::Zand, Grond::Onbekend, Grond::Zand]);
    }

    #[test]
    fn soil_names() {
        assert_eq!(soil_of_name("zand"), Grond::Zand);
        assert_eq!(soil_of_name("zwakSiltigZand"), Grond::Zand);
        assert_eq!(soil_of_name("kleiigZand"), Grond::SiltigZand);
        assert_eq!(soil_of_name("sterkSiltigZand"), Grond::SiltigZand);
        assert_eq!(soil_of_name("matigSiltigeKlei"), Grond::Klei);
        assert_eq!(soil_of_name("zwakZandigeKleiMetGrind"), Grond::Klei);
        assert_eq!(soil_of_name("mineraalarmVeen"), Grond::Veen);
        assert_eq!(soil_of_name("sterkKleiigVeen"), Grond::Veen);
        assert_eq!(soil_of_name("zandigeLeem"), Grond::Silt);
        assert_eq!(soil_of_name("zandigGrind"), Grond::Grind);
        assert_eq!(humanize("zwakSiltigZand"), "zwak siltig zand");
    }

    #[test]
    fn a_borehole_object() {
        // NEN-EN-ISO 14688 names; the field log, not the lab log of samples.
        let b = parse_bhr(&fixture("bhrgt-BHR000000431162.xml")).unwrap();
        assert_eq!(b.point.id, "BHR000000431162");
        assert_eq!((b.point.rd, b.point.surface), (Some([104900.855, 425016.05]), Some(0.564)));
        assert!(b.procedure.starts_with("ISO14688"), "{}", b.procedure);
        assert_eq!(b.layers[0], Interval { top: 0.0, bottom: 0.3, soil: Grond::Zand, name: "zand".into(), qc: None, rf: None });
        assert!(b.layers.iter().any(|l| l.soil == Grond::SiltigZand && l.name == "kleiig zand"));
        assert!(b.layers.len() >= 8, "{:#?}", b.layers);
        // NEN 5104 names (IMBRO/A archive).
        let b = parse_bhr(&fixture("bhrgt-BHR000000449010.xml")).unwrap();
        let soils: std::collections::HashSet<Grond> = b.layers.iter().map(|l| l.soil).collect();
        assert!(soils.contains(&Grond::Veen) && soils.contains(&Grond::Klei), "{:#?}", b.layers);
        assert!(b.layers.windows(2).all(|w| w[1].top >= w[0].top));
    }

    /// Live: the BRO around the Grote Kerk in Dordrecht.
    #[test]
    #[ignore]
    fn live_dordrecht() {
        let b = [4.656, 51.811, 4.665, 51.817];
        let cpts = search(Kind::Cpt, b).unwrap();
        assert!(cpts.len() > 10, "{}", cpts.len());
        let c = fetch_cpt(&cpts[0].id).unwrap();
        assert!(!c.samples.is_empty());
        for l in cpt_intervals(&c.samples, 0.2) {
            eprintln!("{} {:6.2}–{:6.2} m NAP {:<28} qc {:5.2} Rf {:4.1}", c.point.id, c.point.surface.unwrap_or(0.0) - l.top, c.point.surface.unwrap_or(0.0) - l.bottom, l.name, l.qc.unwrap_or(0.0), l.rf.unwrap_or(0.0));
        }
        let bhrs = search(Kind::Bhr, b).unwrap();
        assert!(!bhrs.is_empty());
        let h = fetch_bhr(&bhrs[0].id).unwrap();
        assert!(!h.layers.is_empty());
    }
}
