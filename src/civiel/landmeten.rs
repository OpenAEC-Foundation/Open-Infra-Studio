//! Survey points (Road "GeoPoints"): point files in, points out, points
//! placed with their ground height. Point codes become linework and symbols:
//! a code table maps each code to an NLCS layer, a symbol block and point or
//! line; line codes with their control codes (begin, end, close, arcs, line
//! numbers) give 3D polylines.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use ocs_plugin_api::host::codec::{entities::TextHorizontalAlignment, CadDocument, EntityType, Handle, Line, Point};

use super::{record, v3, Ctx, Maaiveld, Out};
use crate::gis::crs::XY;
use crate::hoogte::nap;
use crate::nlcs::Element;

#[derive(Clone, Debug, PartialEq)]
pub struct SurveyPoint {
    pub name: String,
    /// Easting and northing in project coordinates (m).
    pub e: f64,
    pub n: f64,
    pub z: Option<f64>,
    pub code: String,
}

/// Column order of a point file: P(unt) N(oord) E(ast/oost) Z D(escription).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Auto,
    Pnezd,
    Penzd,
    Nezd,
    Enzd,
}

impl Format {
    pub const ALL: [Format; 5] = [Format::Auto, Format::Penzd, Format::Pnezd, Format::Enzd, Format::Nezd];

    pub fn label(self) -> &'static str {
        match self {
            Format::Auto => "Automatisch",
            Format::Pnezd => "PNEZD (punt, noord, oost, Z, code)",
            Format::Penzd => "PENZD (punt, X, Y, Z, code)",
            Format::Nezd => "NEZD (noord, oost, Z, code)",
            Format::Enzd => "ENZD (X, Y, Z, code)",
        }
    }

    pub fn short(self) -> &'static str {
        match self {
            Format::Auto => "auto",
            Format::Pnezd => "PNEZD",
            Format::Penzd => "PENZD",
            Format::Nezd => "NEZD",
            Format::Enzd => "ENZD",
        }
    }

    fn named(self) -> bool {
        matches!(self, Format::Pnezd | Format::Penzd)
    }

    fn north_first(self) -> bool {
        matches!(self, Format::Pnezd | Format::Nezd)
    }
}

/// A number, with a decimal point or comma.
fn num(t: &str) -> Option<f64> {
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    t.replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Fields of one line: semicolons (decimal commas), tabs, commas or spaces.
fn fields(line: &str) -> Vec<String> {
    let l = line.trim();
    let parts: Vec<&str> = if l.contains(';') {
        l.split(';').collect()
    } else if l.contains('\t') {
        l.split('\t').collect()
    } else if l.contains(',') {
        l.split(',').collect()
    } else {
        l.split_whitespace().collect()
    };
    parts.into_iter().map(|t| t.trim().trim_matches('"').to_string()).collect()
}

/// One line as (name, first coordinate, second, z, code).
fn read_line(f: &[String], format: Format) -> Option<(Option<String>, f64, f64, Option<f64>, String)> {
    let rest = |from: usize| f.get(from..).map(|r| r.join(" ").trim().to_string()).unwrap_or_default();
    let named = match format {
        Format::Auto => {
            // "P c1 c2 [z] [code]" unless the line starts with three numbers
            // and has no fourth one: then it is "c1 c2 z [code]".
            let lead = f.iter().take_while(|t| num(t).is_some()).count();
            !(lead == 3 || (lead >= 3 && f.len() == 3))
        }
        fmt => fmt.named(),
    };
    if named {
        let (a, b) = (num(f.get(1)?)?, num(f.get(2)?)?);
        let z = f.get(3).and_then(|t| num(t));
        let code = if z.is_some() { rest(4) } else { rest(3) };
        Some((Some(f.first()?.clone()), a, b, z, code))
    } else {
        let (a, b) = (num(f.first()?)?, num(f.get(1)?)?);
        let z = f.get(2).and_then(|t| num(t));
        let code = if z.is_some() { rest(3) } else { rest(2) };
        Some((None, a, b, z, code))
    }
}

pub struct Parsed {
    pub points: Vec<SurveyPoint>,
    /// The order that was used (with Auto: the one detected).
    pub format: Format,
    pub skipped: usize,
}

/// Points from a point file. With `Format::Auto` the order of the two
/// coordinates follows the numbers: northings are larger than eastings in
/// RD, UTM and Lambert-93.
pub fn parse(text: &str, format: Format) -> Result<Parsed, String> {
    let mut rows = Vec::new();
    let mut skipped = 0;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with("//") {
            continue;
        }
        match read_line(&fields(l), format) {
            Some(r) => rows.push(r),
            None => skipped += 1,
        }
    }
    if rows.is_empty() {
        return Err("geen punten gevonden (verwacht: puntnummer, X, Y, Z, code per regel)".into());
    }
    let named = rows.iter().filter(|r| r.0.is_some()).count() * 2 >= rows.len();
    let north_first = match format {
        Format::Auto => rows.iter().filter(|r| r.1 > r.2).count() * 2 > rows.len(),
        f => f.north_first(),
    };
    let used = match (format, named, north_first) {
        (Format::Auto, true, true) => Format::Pnezd,
        (Format::Auto, true, false) => Format::Penzd,
        (Format::Auto, false, true) => Format::Nezd,
        (Format::Auto, false, false) => Format::Enzd,
        (f, ..) => f,
    };
    let points = rows
        .into_iter()
        .enumerate()
        .map(|(i, (name, a, b, z, code))| {
            let (e, n) = if north_first { (b, a) } else { (a, b) };
            SurveyPoint { name: name.unwrap_or_else(|| (i + 1).to_string()), e, n, z, code }
        })
        .collect();
    Ok(Parsed { points, format: used, skipped })
}

/// The point file text: comma separated, decimal points (Road).
pub fn write(points: &[SurveyPoint], format: Format) -> String {
    let format = if format == Format::Auto { Format::Penzd } else { format };
    let mut out = String::new();
    for p in points {
        let (a, b) = if format.north_first() { (p.n, p.e) } else { (p.e, p.n) };
        let z = p.z.map(|z| format!("{z:.3}")).unwrap_or_default();
        let code = p.code.replace(',', " ");
        if format.named() {
            out.push_str(&format!("{},{a:.3},{b:.3},{z},{code}\n", p.name.replace(',', " ")));
        } else {
            out.push_str(&format!("{a:.3},{b:.3},{z},{code}\n"));
        }
    }
    out
}

/// Survey points in the drawing on OG-HOOGTEPUNT: a point and a cross at
/// the true height (S), number, height and code beside it (T).
pub fn draw(ctx: &Ctx, points: &[SurveyPoint], source: &str, out: &mut Out) -> Result<usize, String> {
    let s_spec = ctx.layer("OG", "HOOGTEPUNT", Element::S)?;
    let t_spec = ctx.layer("OG", "HOOGTEPUNT", Element::T)?;
    let (f, k) = (ctx.f, ctx.k);
    for p in points {
        let at = ctx.to_drawing([p.e, p.n]);
        let z = p.z.unwrap_or(0.0) * f;
        let mut pairs = vec![("puntnummer", p.name.clone()), ("x", format!("{:.3}", p.e)), ("y", format!("{:.3}", p.n))];
        if let Some(h) = p.z {
            pairs.push(("z", format!("{h:.3}")));
        }
        if !p.code.is_empty() {
            pairs.push(("code", p.code.clone()));
        }
        if !source.is_empty() {
            pairs.push(("bron", source.to_string()));
        }
        let d = record(&pairs);
        let mut symbol = Vec::new();
        let mut pt = EntityType::Point(Point::from_coords(at[0], at[1], z));
        pt.common_mut().extended_data.add_record(d.clone());
        symbol.push(pt);
        let arm = 0.8 * k;
        for (a, b) in [([at[0] - arm, at[1]], [at[0] + arm, at[1]]), ([at[0], at[1] - arm], [at[0], at[1] + arm])] {
            symbol.push(EntityType::Line(Line::from_points(v3(a, z), v3(b, z))));
        }
        for e in symbol.iter_mut() {
            let c = e.common_mut();
            c.layer = s_spec.name.clone();
            c.color = ocs_plugin_api::host::codec::Color::ByLayer;
        }
        out.put(&s_spec, symbol);
        if ctx.opt.point_labels {
            let left = |mut t: EntityType| {
                if let EntityType::Text(tx) = &mut t {
                    tx.horizontal_alignment = TextHorizontalAlignment::Left;
                    tx.insertion_point.z = z;
                    if let Some(a) = tx.alignment_point.as_mut() {
                        a.z = z;
                    }
                }
                t
            };
            let mut labels = vec![left(ctx.text([at[0] + 1.2 * k, at[1] + 1.4 * k], &p.name, 1.8, 0.0, &t_spec, None))];
            if let Some(h) = p.z {
                labels.push(left(ctx.text([at[0] + 1.2 * k, at[1] - 1.4 * k], &nap(h, 2), 1.8, 0.0, &t_spec, None)));
            }
            if !p.code.is_empty() {
                labels.push(left(ctx.text([at[0] + 1.2 * k, at[1] - 4.0 * k], &p.code, 1.8, 0.0, &t_spec, None)));
            }
            out.put(&t_spec, labels);
        }
    }
    Ok(points.len())
}

/// Register the drawn points (the first of every three entities: point and
/// two cross lines) in the drawing's object index.
pub fn index_points(handles: &[Handle], n: usize) {
    let points: Vec<Handle> = handles.iter().step_by(3).take(n).copied().collect();
    crate::bridge::index_add("punten", "", &points, false);
}

/// Import a point file: parse, draw, one undo step.
pub fn import(path: &std::path::Path, opt: &super::CivielOptions) -> Result<String, String> {
    import_coded(path, opt, None)
}

/// Import a point file; with `codes` the point codes also become linework
/// and symbols (a second undo step).
pub fn import_coded(path: &std::path::Path, opt: &super::CivielOptions, codes: Option<CodeOptions>) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let parsed = parse(&text, opt.format)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut out = Out::default();
    let n = draw(&ctx, &parsed.points, &name, &mut out)?;
    let handles = out.write_handles(&format!("Punten {name}"))?;
    let written = handles.len();
    index_points(&handles, n);
    super::api::note("punten", n);
    super::api::note("formaat", parsed.format.short());
    super::api::note("overgeslagen", parsed.skipped);
    let zs: Vec<f64> = parsed.points.iter().filter_map(|p| p.z).collect();
    let range = if zs.is_empty() {
        String::new()
    } else {
        let (lo, hi) = zs.iter().fold((f64::MAX, f64::MIN), |(a, b), z| (a.min(*z), b.max(*z)));
        format!(", {} tot {} m", nap(lo, 2), nap(hi, 2))
    };
    let coded = match codes {
        Some(c) => {
            let table = CodeTable::load(c.table.as_deref())?;
            format!(" {}", apply_codes(&ctx, &table, &parsed.points, &name, false)?)
        }
        None => String::new(),
    };
    Ok(format!(
        "{n} punten uit {name} ({}{range}){} — {written} elementen{}.{coded}",
        parsed.format.short(),
        if parsed.skipped > 0 { format!(", {} regels overgeslagen", parsed.skipped) } else { String::new() },
        if ctx.site.is_none() { "; zonder projectlocatie: coördinaten zoals ze zijn" } else { "" }
    ))
}

/// The points of the drawing (the selected ones, when there are) as survey
/// points in project coordinates.
pub fn collect(doc: &CadDocument, ctx: &Ctx, selection: &HashSet<Handle>) -> Vec<SurveyPoint> {
    let mut out = Vec::new();
    for e in doc.model_space_entities() {
        let EntityType::Point(p) = e else { continue };
        if !selection.is_empty() && !selection.contains(&p.common.handle) {
            continue;
        }
        let q = ctx.from_drawing([p.location.x, p.location.y]);
        let name = super::data_value(e, "puntnummer").unwrap_or_else(|| (out.len() + 1).to_string());
        let code = super::data_value(e, "code").unwrap_or_default();
        out.push(SurveyPoint { name, e: q[0], n: q[1], z: Some(p.location.z / ctx.f), code });
    }
    out
}

/// Write the drawing's points to `path`, or beside the drawing.
pub fn export(opt: &super::CivielOptions, path: Option<std::path::PathBuf>) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let doc = super::document(&["punten"], &[])?;
    let sel: HashSet<Handle> = crate::bridge::selection().unwrap_or_default().into_iter().collect();
    let points = collect(&doc, &ctx, &sel);
    if points.is_empty() {
        return Err("geen punten (POINT) in de tekening of de selectie".into());
    }
    let path = path.unwrap_or_else(|| crate::bridge::beside_drawing("punten.csv"));
    let format = if opt.format == Format::Auto { Format::Penzd } else { opt.format };
    std::fs::write(&path, write(&points, format)).map_err(|e| format!("{}: {e}", path.display()))?;
    super::api::note("punten", points.len());
    super::api::note("bestand", path.to_string_lossy().into_owned());
    Ok(format!("{} punten ({}) opgeslagen in {}.", points.len(), format.short(), path.display()))
}

/// The next free point number in the drawing: one above the highest.
pub fn next_number(doc: &CadDocument) -> usize {
    doc.model_space_entities()
        .filter(|e| matches!(e, EntityType::Point(_)))
        .filter_map(|e| super::data_value(e, "puntnummer"))
        .filter_map(|n| n.trim().parse::<usize>().ok())
        .max()
        .map(|n| n + 1)
        .unwrap_or(1)
}

/// A point placed by clicking: its ground height from the TIN or AHN.
pub struct Placer {
    ctx: Ctx,
    maaiveld: Maaiveld,
    next: usize,
}

impl Placer {
    pub fn new(opt: &super::CivielOptions) -> Result<Placer, String> {
        let ctx = Ctx::new(opt)?;
        let doc = super::document(&["tin", "punten"], &[])?;
        let maaiveld = Maaiveld::find(&ctx, &doc)?;
        Ok(Placer { next: next_number(&doc), ctx, maaiveld })
    }

    pub fn place(&mut self, at: XY) -> Result<String, String> {
        self.place_with(at, None, None, String::new())
    }

    /// A point at a drawing position; without a height, the ground's.
    pub fn place_with(&mut self, at: XY, z: Option<f64>, name: Option<String>, code: String) -> Result<String, String> {
        let z = z.or_else(|| self.maaiveld.height(at, self.ctx.f));
        let q = self.ctx.from_drawing(at);
        let p = SurveyPoint { name: name.unwrap_or_else(|| self.next.to_string()), e: q[0], n: q[1], z, code };
        let mut out = Out::default();
        draw(&self.ctx, std::slice::from_ref(&p), &self.maaiveld.label(), &mut out)?;
        let handles = out.write_handles(&format!("Punt {}", p.name))?;
        index_points(&handles, 1);
        self.maaiveld.note_source();
        self.next += 1;
        Ok(match z {
            Some(h) => format!("Punt {}: {} m ({}).", p.name, nap(h, 2), self.maaiveld.label()),
            None => format!("Punt {}: geen hoogte op deze plek ({}).", p.name, self.maaiveld.error().unwrap_or_else(|| self.maaiveld.label())),
        })
    }
}

// ── Codes: linework and symbols ─────────────────────────────────────────────

/// A message in the interface language (Dutch or English).
macro_rules! t {
    ($nl:literal, $en:literal $(, $arg:expr)* $(,)?) => {
        if crate::lang::current() == crate::lang::Lang::Nl { format!($nl $(, $arg)*) } else { format!($en $(, $arg)*) }
    };
}

/// Control codes after a line code: separate words (`LTRB1 B`, as Dutch field
/// software writes them) or glued on with `scheiding` (`LTRB1/B`).
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Stuurcodes {
    /// Begin a string; an open string of the same code and number ends first.
    #[serde(default)]
    pub begin: Vec<String>,
    /// This point ends the string.
    #[serde(default)]
    pub einde: Vec<String>,
    /// This point ends the string and closes it to its first point.
    #[serde(default)]
    pub sluiten: Vec<String>,
    /// Continue (what a line code does anyway).
    #[serde(default)]
    pub vervolg: Vec<String>,
    /// Begin an arc here: through the next two points, or up to `einde_boog`.
    #[serde(default)]
    pub boog: Vec<String>,
    #[serde(default)]
    pub einde_boog: Vec<String>,
    /// A point on an arc through its neighbours.
    #[serde(default)]
    pub boogpunt: Vec<String>,
    #[serde(default)]
    pub scheiding: String,
    /// Digits right after a code are a line number: LTRB1 and LTRB2 are two strings.
    #[serde(default = "yes")]
    pub lijnnummers: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodeDef {
    pub code: String,
    /// "punt" (a symbol), "lijn" (a 3D polyline), "tekst" (its parameters as
    /// a text) or "negeren" (a sighting point and the like).
    pub soort: String,
    #[serde(default)]
    pub omschrijving: String,
    /// NLCS hoofdgroep and object, "VH KANTOPSLUITING_TROTTOIRBAND".
    #[serde(default)]
    pub nlcs: Option<String>,
    /// A plain layer name instead of an NLCS object.
    #[serde(default)]
    pub laag: Option<String>,
    /// Symbol block (point codes): boom, put, kolk, lichtmast, paal, bord,
    /// brandkraan, afsluiter, kast, verkeerslicht, waterpeil, kruis, cirkel,
    /// vierkant, driehoek.
    #[serde(default)]
    pub symbool: Option<String>,
    /// Names of the parameters after the code ("SBM Eik 70": soort, diameter).
    #[serde(default)]
    pub attributen: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeKind {
    Point,
    Line,
    Text,
    Ignore,
}

impl CodeDef {
    pub fn kind(&self) -> CodeKind {
        match self.soort.to_ascii_lowercase().as_str() {
            "lijn" | "line" => CodeKind::Line,
            "tekst" | "text" => CodeKind::Text,
            "negeren" | "ignore" => CodeKind::Ignore,
            _ => CodeKind::Point,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CodeTable {
    #[serde(default)]
    pub versie: u32,
    #[serde(default)]
    pub toelichting: String,
    pub stuurcodes: Stuurcodes,
    pub codes: Vec<CodeDef>,
    /// Where the table came from (not in the file).
    #[serde(skip)]
    pub bron: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Begin,
    End,
    Close,
    Continue,
    ArcBegin,
    ArcEnd,
    ArcPoint,
}

/// One point's code read against the table.
#[derive(Clone, Debug, PartialEq)]
pub struct Coded {
    /// Index into `CodeTable::codes`.
    pub def: usize,
    /// The line number ("" without one).
    pub line: String,
    pub controls: Vec<Control>,
    /// The other words: attributes, a text.
    pub params: Vec<String>,
}

/// Where code tables are read from: an explicit file, else the user copy.
#[derive(Clone, Debug, Default)]
pub struct CodeOptions {
    pub table: Option<std::path::PathBuf>,
}

pub const BUILTIN_CODES: &str = include_str!("../../assets/landmeten/codes.json");

impl CodeTable {
    pub fn builtin() -> CodeTable {
        let mut t: CodeTable = serde_json::from_str(BUILTIN_CODES).expect("assets/landmeten/codes.json");
        t.bron = crate::lang::tr("ingebouwde codetabel", "built-in code table").into();
        t
    }

    pub fn parse(text: &str, bron: &str) -> Result<CodeTable, String> {
        let mut t: CodeTable = serde_json::from_str(text).map_err(|e| t!("codetabel {bron} onleesbaar: {e}", "code table {bron} unreadable: {e}"))?;
        if t.codes.is_empty() {
            return Err(t!("codetabel {bron} heeft geen codes", "code table {bron} has no codes"));
        }
        t.bron = bron.to_string();
        Ok(t)
    }

    /// The user copy: `<config>/OpenCADStudio/open-infra/landmeten/codes.json`.
    pub fn user_path() -> std::path::PathBuf {
        crate::gis::servers::config_dir().join("landmeten").join("codes.json")
    }

    /// `path`, else the user copy; the first time the built-in table is
    /// written there to be edited.
    pub fn load(path: Option<&std::path::Path>) -> Result<CodeTable, String> {
        let user = Self::user_path();
        let file = match path {
            Some(p) => p.to_path_buf(),
            None if user.exists() => user,
            None => {
                if let Some(dir) = user.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&user, BUILTIN_CODES);
                return Ok(Self::builtin());
            }
        };
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        Self::parse(&text, &file.display().to_string())
    }

    fn control(&self, word: &str) -> Option<Control> {
        let c = &self.stuurcodes;
        let is = |list: &[String]| list.iter().any(|t| t.eq_ignore_ascii_case(word));
        if is(&c.begin) {
            Some(Control::Begin)
        } else if is(&c.einde) {
            Some(Control::End)
        } else if is(&c.sluiten) {
            Some(Control::Close)
        } else if is(&c.vervolg) {
            Some(Control::Continue)
        } else if is(&c.boog) {
            Some(Control::ArcBegin)
        } else if is(&c.einde_boog) {
            Some(Control::ArcEnd)
        } else if is(&c.boogpunt) {
            Some(Control::ArcPoint)
        } else {
            None
        }
    }

    /// The code of a point description: the first word is the code (with
    /// its line number and glued control codes), the others are control codes
    /// or parameters. A code that is not in the table gives `None`.
    pub fn read(&self, desc: &str) -> Option<Coded> {
        let mut words = desc.split_whitespace();
        let first = words.next()?;
        let sep = self.stuurcodes.scheiding.as_str();
        let mut glued: Vec<&str> = if sep.is_empty() { vec![first] } else { first.split(sep).filter(|w| !w.is_empty()).collect() };
        if glued.is_empty() {
            return None;
        }
        let token = glued.remove(0);
        let (def, line) = match self.codes.iter().position(|c| c.code.eq_ignore_ascii_case(token)) {
            Some(i) => (i, String::new()),
            None if self.stuurcodes.lijnnummers => {
                // The longest code with only digits after it: LTR131 is LTR13, line 1.
                let up = token.to_ascii_uppercase();
                let (i, c) = self
                    .codes
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| {
                        let k = c.code.to_ascii_uppercase();
                        up.len() > k.len() && up.starts_with(&k) && up[k.len()..].chars().all(|ch| ch.is_ascii_digit())
                    })
                    .max_by_key(|(_, c)| c.code.len())?;
                (i, token[c.code.len()..].to_string())
            }
            None => return None,
        };
        let mut controls = Vec::new();
        let mut params = Vec::new();
        for w in glued.into_iter().chain(words) {
            match self.control(w) {
                Some(c) => controls.push(c),
                None => params.push(w.to_string()),
            }
        }
        Some(Coded { def, line, controls, params })
    }
}

/// How a vertex of a figure joins its neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArcMark {
    None,
    Begin,
    End,
    On,
}

#[derive(Clone, Debug)]
pub struct Figure {
    pub def: usize,
    pub line: String,
    /// Positions (easting, northing, height in m) and arc marks.
    pub nodes: Vec<([f64; 3], ArcMark)>,
    pub names: Vec<String>,
    pub closed: bool,
}

/// What the codes of a series of points make.
#[derive(Clone, Debug, Default)]
pub struct Linework {
    pub figures: Vec<Figure>,
    /// (code, point index, parameters).
    pub symbols: Vec<(usize, usize, Vec<String>)>,
    pub texts: Vec<(usize, usize, Vec<String>)>,
    /// Codes not in the table, with how often they occur.
    pub unknown: std::collections::BTreeMap<String, usize>,
    /// Line strings of a single point (left out).
    pub single: usize,
}

/// The figures and symbols of points in measuring order: a line code
/// continues the open string of its code and line number, or starts one.
pub fn linework(table: &CodeTable, points: &[SurveyPoint]) -> Linework {
    let mut lw = Linework::default();
    let mut open: HashMap<(usize, String), usize> = HashMap::new();
    let mut done: Vec<bool> = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let Some(c) = table.read(&p.code) else {
            if let Some(w) = p.code.split_whitespace().next() {
                *lw.unknown.entry(w.to_string()).or_default() += 1;
            }
            continue;
        };
        match table.codes[c.def].kind() {
            CodeKind::Ignore => {}
            CodeKind::Point => lw.symbols.push((c.def, i, c.params)),
            CodeKind::Text => lw.texts.push((c.def, i, c.params)),
            CodeKind::Line => {
                let key = (c.def, c.line.clone());
                let has = |k: Control| c.controls.contains(&k);
                if has(Control::Begin) {
                    open.remove(&key);
                }
                let f = match open.get(&key) {
                    Some(&f) => f,
                    None => {
                        lw.figures.push(Figure { def: c.def, line: c.line.clone(), nodes: Vec::new(), names: Vec::new(), closed: false });
                        done.push(false);
                        open.insert(key.clone(), lw.figures.len() - 1);
                        lw.figures.len() - 1
                    }
                };
                let mark = if has(Control::ArcBegin) {
                    ArcMark::Begin
                } else if has(Control::ArcEnd) {
                    ArcMark::End
                } else if has(Control::ArcPoint) {
                    ArcMark::On
                } else {
                    ArcMark::None
                };
                let fig = &mut lw.figures[f];
                fig.nodes.push(([p.e, p.n, p.z.unwrap_or(0.0)], mark));
                fig.names.push(p.name.clone());
                if has(Control::Close) {
                    fig.closed = true;
                    open.remove(&key);
                } else if has(Control::End) {
                    open.remove(&key);
                }
            }
        }
    }
    let before = lw.figures.len();
    lw.figures.retain(|f| f.nodes.len() >= 2);
    lw.single = before - lw.figures.len();
    lw
}

/// Points on the circle through `a`, `b`, `c` (plan), from just after `a`
/// to `c`, through `b`; heights linear along each half. Straight when the
/// three lie on a line.
pub fn arc3(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> Vec<[f64; 3]> {
    let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
    let span = (c[0] - a[0]).hypot(c[1] - a[1]).max(1e-9);
    if cross.abs() < 1e-6 * span * span {
        return vec![b, c];
    }
    // Circumcentre.
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    let sq = |p: [f64; 3]| p[0] * p[0] + p[1] * p[1];
    let ux = (sq(a) * (b[1] - c[1]) + sq(b) * (c[1] - a[1]) + sq(c) * (a[1] - b[1])) / d;
    let uy = (sq(a) * (c[0] - b[0]) + sq(b) * (a[0] - c[0]) + sq(c) * (b[0] - a[0])) / d;
    let r = (a[0] - ux).hypot(a[1] - uy);
    let ang = |p: [f64; 3]| (p[1] - uy).atan2(p[0] - ux);
    let ccw = cross > 0.0;
    let sweep = |from: f64, to: f64| {
        let mut s = to - from;
        if ccw {
            while s <= 0.0 {
                s += std::f64::consts::TAU;
            }
        } else {
            while s >= 0.0 {
                s -= std::f64::consts::TAU;
            }
        }
        s
    };
    let mut out = Vec::new();
    for (p, q) in [(a, b), (b, c)] {
        let (a0, s) = (ang(p), sweep(ang(p), ang(q)));
        // Chords of at most 5° and at most 0.5 m.
        let n = ((s.abs() / 5f64.to_radians()).ceil() as usize).max(((s.abs() * r) / 0.5).ceil() as usize).clamp(1, 360);
        for k in 1..=n {
            let t = k as f64 / n as f64;
            if k == n {
                out.push(q);
            } else {
                let th = a0 + s * t;
                out.push([ux + r * th.cos(), uy + r * th.sin(), p[2] + (q[2] - p[2]) * t]);
            }
        }
    }
    out
}

impl Figure {
    /// The vertices with arcs made into short chords.
    pub fn points(&self) -> Vec<[f64; 3]> {
        let n = self.nodes.len();
        let p = |i: usize| self.nodes[i].0;
        let mut out: Vec<[f64; 3]> = Vec::with_capacity(n);
        let push = |out: &mut Vec<[f64; 3]>, q: [f64; 3]| {
            if out.last() != Some(&q) {
                out.push(q);
            }
        };
        let mut i = 0;
        while i < n {
            let (q, mark) = self.nodes[i];
            push(&mut out, q);
            match mark {
                ArcMark::Begin if i + 2 < n || (i + 1 < n && self.nodes[i + 1].1 == ArcMark::End) => {
                    // To the closing EC, or through the next two points.
                    let end = (i + 1..n).find(|&j| self.nodes[j].1 == ArcMark::End).unwrap_or(i + 2).min(n - 1);
                    let mut k = i;
                    while k + 2 <= end {
                        for v in arc3(p(k), p(k + 1), p(k + 2)) {
                            push(&mut out, v);
                        }
                        k += 2;
                    }
                    if k < end {
                        push(&mut out, p(end));
                    }
                    // The last point may begin the next arc.
                    if self.nodes[end].1 == ArcMark::Begin && end > i {
                        i = end;
                        out.pop();
                        continue;
                    }
                    i = end + 1;
                }
                ArcMark::On if i > 0 && i + 1 < n => {
                    out.pop();
                    for v in arc3(p(i - 1), q, p(i + 1)) {
                        push(&mut out, v);
                    }
                    i += 2;
                }
                _ => i += 1,
            }
        }
        out
    }
}

/// The layer of a code: its NLCS object, or its plain layer.
fn code_layer(ctx: &Ctx, def: &CodeDef, el: Element) -> Result<crate::nlcs::LayerSpec, String> {
    if let Some(name) = def.laag.as_deref().filter(|n| !n.trim().is_empty()) {
        return Ok(crate::nlcs::LayerSpec {
            name: name.trim().to_string(),
            color: 7,
            rgb: crate::nlcs::color_rgb(7),
            true_color: None,
            lineweight: None,
            linetype: "CONTINUOUS".into(),
            object_id: 0,
        });
    }
    let nlcs = def.nlcs.as_deref().unwrap_or("OG TOPOGRAFIE");
    let (group, name) = nlcs.trim().split_once(char::is_whitespace).ok_or_else(|| t!("code {}: nlcs \"{nlcs}\" moet \"HOOFDGROEP OBJECT\" zijn", "code {}: nlcs \"{nlcs}\" must be \"GROUP OBJECT\"", def.code))?;
    ctx.layer(group.trim(), name.trim(), el)
}

/// Symbol shapes in paper millimetres around the insertion point.
pub enum SymbolShape {
    Circle([f64; 2], f64),
    Poly(Vec<[f64; 2]>, bool),
}

pub const SYMBOLS: &[&str] = &["boom", "put", "kolk", "lichtmast", "paal", "bord", "brandkraan", "afsluiter", "kast", "verkeerslicht", "waterpeil", "kruis", "cirkel", "vierkant", "driehoek"];

pub fn symbol_shapes(name: &str) -> Vec<SymbolShape> {
    use SymbolShape::*;
    let rect = |w: f64, h: f64| Poly(vec![[-w / 2.0, -h / 2.0], [w / 2.0, -h / 2.0], [w / 2.0, h / 2.0], [-w / 2.0, h / 2.0]], true);
    let line = |a: [f64; 2], b: [f64; 2]| Poly(vec![a, b], false);
    match name.to_ascii_lowercase().as_str() {
        // A crown with the trunk as a dot.
        "boom" => vec![Circle([0.0, 0.0], 1.5), Circle([0.0, 0.0], 0.15)],
        "put" => vec![Circle([0.0, 0.0], 1.0), line([-1.0, 0.0], [1.0, 0.0])],
        "kolk" => vec![rect(1.6, 0.8), line([-0.8, -0.4], [0.8, 0.4])],
        "lichtmast" => vec![Circle([0.0, 0.0], 0.5), line([0.5, 0.0], [1.8, 0.0]), Circle([2.1, 0.0], 0.3)],
        "paal" => vec![Circle([0.0, 0.0], 0.4)],
        "bord" => vec![line([0.0, 0.0], [0.0, 1.2]), Poly(vec![[-0.8, 1.2], [0.8, 1.2], [0.0, 2.6]], true)],
        "brandkraan" => vec![Circle([0.0, 0.0], 1.0), line([-0.5, -0.6], [-0.5, 0.6]), line([0.5, -0.6], [0.5, 0.6]), line([-0.5, 0.0], [0.5, 0.0])],
        "afsluiter" => vec![rect(1.2, 1.2), line([-0.6, -0.6], [0.6, 0.6]), line([-0.6, 0.6], [0.6, -0.6])],
        "kast" => vec![rect(1.6, 1.0), line([-0.8, -0.5], [0.8, 0.5])],
        "verkeerslicht" => vec![rect(0.8, 2.0), Circle([0.0, 0.6], 0.25), Circle([0.0, 0.0], 0.25), Circle([0.0, -0.6], 0.25)],
        "waterpeil" => vec![Poly(vec![[0.0, 0.0], [0.8, 1.2], [-0.8, 1.2]], true), line([-1.2, 0.0], [1.2, 0.0])],
        "kruis" => vec![line([-0.8, 0.0], [0.8, 0.0]), line([0.0, -0.8], [0.0, 0.8])],
        "vierkant" => vec![rect(1.2, 1.2)],
        "driehoek" => vec![Poly(vec![[-0.7, -0.5], [0.7, -0.5], [0.0, 0.8]], true)],
        _ => vec![Circle([0.0, 0.0], 0.8)],
    }
}

/// The block of a symbol, defined once per drawing (layer 0, ByBlock, in
/// paper millimetres: inserts scale it to the drawing scale).
fn ensure_symbol(name: &str) -> Result<String, String> {
    use ocs_plugin_api::host::codec::{Circle, Color, LineWeight, LwPolyline, Vector2, Vector3};
    use ocs_plugin_api::host::TableOperation;
    use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
    static MADE: std::sync::Mutex<Option<HashSet<(u64, String)>>> = std::sync::Mutex::new(None);
    let shape = if SYMBOLS.contains(&name.to_ascii_lowercase().as_str()) { name.to_ascii_lowercase() } else { "cirkel".into() };
    let block = format!("OI_SYMBOOL_{}", shape.to_ascii_uppercase());
    let t = crate::bridge::target().ok_or_else(|| crate::lang::tr("geen tekening actief", "no drawing active").to_string())?;
    let key = (t.tab_id, block.clone());
    if MADE.lock().unwrap().get_or_insert_with(HashSet::new).contains(&key) {
        return Ok(block);
    }
    let table = |op: TableOperation| -> Result<(), String> {
        match t.sender.request(PluginRequest::TableOperation { operation: op }) {
            Ok(PluginResponse::TableResult(Ok(_))) => Ok(()),
            Ok(PluginResponse::TableResult(Err(e))) | Ok(PluginResponse::Error(e)) => Err(e),
            Ok(other) => Err(format!("{other:?}")),
            Err(e) => Err(e.to_string()),
        }
    };
    let description = t!("Open Infra meetsymbool {shape} (papiermillimeters)", "Open Infra survey symbol {shape} (paper millimetres)");
    let exists = table(TableOperation::BlockModify { name: block.clone(), description: Some(description.clone()), explodable: Some(true), scale_uniformly: Some(true) }).is_ok();
    if !exists {
        let geometry: Vec<EntityType> = symbol_shapes(&shape)
            .into_iter()
            .map(|s| {
                let mut e = match s {
                    SymbolShape::Circle(c, r) => EntityType::Circle(Circle::from_center_radius(Vector3::new(c[0], c[1], 0.0), r)),
                    SymbolShape::Poly(pts, closed) => {
                        let mut lw = LwPolyline::new();
                        for p in pts {
                            lw.add_point_with_bulge(Vector2::new(p[0], p[1]), 0.0);
                        }
                        lw.is_closed = closed;
                        EntityType::LwPolyline(lw)
                    }
                };
                let c = e.common_mut();
                c.layer = "0".into();
                c.color = Color::ByBlock;
                c.line_weight = LineWeight::ByBlock;
                e
            })
            .collect();
        let handles = crate::bridge::add_entities_handles(&format!("Open Infra: {block}"), geometry)?;
        table(TableOperation::BlockCreate { name: block.clone(), entities: handles, base_point: [0.0, 0.0, 0.0], erase_originals: true, description: Some(description) })?;
    }
    MADE.lock().unwrap().get_or_insert_with(HashSet::new).insert(key);
    Ok(block)
}

/// Counts of what the codes drew.
#[derive(Debug, Default)]
pub struct CodeCounts {
    pub lines: usize,
    pub symbols: usize,
    pub texts: usize,
}

/// The linework, symbols and texts of `lw` into `out`.
pub fn draw_codes(ctx: &Ctx, table: &CodeTable, lw: &Linework, points: &[SurveyPoint], source: &str, out: &mut Out) -> Result<CodeCounts, String> {
    use ocs_plugin_api::host::codec::entities::{Insert, Polyline3D};
    let f = ctx.f;
    let mut n = CodeCounts::default();
    for fig in &lw.figures {
        let def = &table.codes[fig.def];
        let spec = code_layer(ctx, def, Element::G)?;
        let pts = fig.points();
        let mut pl = Polyline3D::from_points(pts.iter().map(|p| v3(ctx.to_drawing([p[0], p[1]]), p[2] * f)).collect());
        pl.flags.closed = fig.closed;
        let mut e = EntityType::Polyline3D(pl);
        let mut pairs = vec![("code", def.code.clone()), ("omschrijving", def.omschrijving.clone()), ("punten", fig.names.join(";"))];
        if !fig.line.is_empty() {
            pairs.push(("lijnnummer", fig.line.clone()));
        }
        if !source.is_empty() {
            pairs.push(("bron", source.to_string()));
        }
        let c = e.common_mut();
        c.layer = spec.name.clone();
        c.color = ocs_plugin_api::host::codec::Color::ByLayer;
        c.extended_data.add_record(record(&pairs));
        out.put(&spec, [e]);
        n.lines += 1;
    }
    let attributes = |def: &CodeDef, params: &[String]| -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = def.attributen.iter().zip(params).map(|(k, p)| (k.clone(), p.clone())).collect();
        if params.len() > def.attributen.len() {
            v.push(("parameters".into(), params[def.attributen.len()..].join(" ")));
        }
        v
    };
    for (d, i, params) in &lw.symbols {
        let def = &table.codes[*d];
        let p = &points[*i];
        let spec = code_layer(ctx, def, Element::S)?;
        let block = ensure_symbol(def.symbool.as_deref().unwrap_or("cirkel"))?;
        let at = ctx.to_drawing([p.e, p.n]);
        let ins = Insert::new(block, v3(at, p.z.unwrap_or(0.0) * f)).with_scale(ctx.k, ctx.k, ctx.k);
        let mut e = EntityType::Insert(ins);
        let attrs = attributes(def, params);
        let mut pairs: Vec<(&str, String)> = vec![("code", def.code.clone()), ("omschrijving", def.omschrijving.clone()), ("puntnummer", p.name.clone())];
        pairs.extend(attrs.iter().map(|(k, v)| (k.as_str(), v.clone())));
        let c = e.common_mut();
        c.layer = spec.name.clone();
        c.color = ocs_plugin_api::host::codec::Color::ByLayer;
        c.extended_data.add_record(record(&pairs));
        out.put(&spec, [e]);
        n.symbols += 1;
    }
    for (d, i, params) in &lw.texts {
        let def = &table.codes[*d];
        let p = &points[*i];
        if params.is_empty() {
            continue;
        }
        let spec = code_layer(ctx, def, Element::T)?;
        let at = ctx.to_drawing([p.e, p.n]);
        let d = record(&[("code", def.code.clone()), ("puntnummer", p.name.clone())]);
        let mut t = ctx.text(at, &params.join(" "), 1.8, 0.0, &spec, Some(&d));
        if let EntityType::Text(tx) = &mut t {
            tx.horizontal_alignment = TextHorizontalAlignment::Left;
        }
        out.put(&spec, [t]);
        n.texts += 1;
    }
    Ok(n)
}

/// Linework and symbols from the codes of `points` (measuring order), one
/// undo step; the handles go into the drawing's index ("codes"), so
/// `CIV_CODES_TOEPASSEN` can replace them. `replace` removes the earlier
/// code output first.
pub fn apply_codes(ctx: &Ctx, table: &CodeTable, points: &[SurveyPoint], source: &str, replace: bool) -> Result<String, String> {
    let lw = linework(table, points);
    let mut out = Out::default();
    let n = draw_codes(ctx, table, &lw, points, source, &mut out)?;
    let mut removed = 0;
    if replace {
        let old: Vec<Handle> = crate::bridge::index_read().into_iter().filter(|(k, _, _)| k == "codes").flat_map(|(_, _, h)| h).collect();
        removed = crate::bridge::remove_entities(crate::lang::tr("Codes opnieuw", "Codes again"), &old)?;
    }
    let handles = if out.is_empty() { Vec::new() } else { out.write_handles(&format!("Codes {source}"))? };
    crate::bridge::index_add("codes", "", &handles, replace);
    let mut per_code: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for f in &lw.figures {
        *per_code.entry(table.codes[f.def].code.as_str()).or_default() += 1;
    }
    super::api::note("lijnen", n.lines);
    super::api::note("symbolen", n.symbols);
    super::api::note("teksten", n.texts);
    super::api::note("codetabel", table.bron.clone());
    super::api::note("onbekende_codes", serde_json::to_value(&lw.unknown).unwrap_or_default());
    super::api::note("lijnen_per_code", serde_json::to_value(&per_code).unwrap_or_default());
    let unknown = if lw.unknown.is_empty() {
        String::new()
    } else {
        let list = lw.unknown.iter().map(|(c, k)| format!("{c} ({k}×)")).collect::<Vec<_>>().join(", ");
        t!("; niet in de codetabel: {list}", "; not in the code table: {list}")
    };
    let single = if lw.single > 0 { t!(", {} lijn(en) van één punt overgeslagen", ", {} single-point line(s) skipped", lw.single) } else { String::new() };
    let replaced = if removed > 0 { t!(", {removed} oude elementen vervangen", ", {removed} old elements replaced") } else { String::new() };
    Ok(t!(
        "Codes ({}): {} lijnen, {} symbolen, {} teksten{single}{replaced}{unknown}.",
        "Codes ({}): {} lines, {} symbols, {} texts{single}{replaced}{unknown}.",
        table.bron,
        n.lines,
        n.symbols,
        n.texts
    ))
}

/// `CIV_CODES_TOEPASSEN`: the codes of the points in the drawing (the
/// selected ones, when there are) again, in the order they were made. With
/// all points the earlier code output is replaced.
pub fn apply_in_drawing(opt: &super::CivielOptions, codes: CodeOptions) -> Result<String, String> {
    let ctx = Ctx::new(opt)?;
    let table = CodeTable::load(codes.table.as_deref())?;
    let doc = super::document(&["punten"], &[])?;
    let sel: HashSet<Handle> = crate::bridge::selection().unwrap_or_default().into_iter().collect();
    let mut found: Vec<(u64, SurveyPoint)> = Vec::new();
    for e in doc.model_space_entities() {
        let EntityType::Point(p) = e else { continue };
        let h = p.common.handle;
        if (!sel.is_empty() && !sel.contains(&h)) || super::data_value(e, "code").is_none() {
            continue;
        }
        let q = ctx.from_drawing([p.location.x, p.location.y]);
        let name = super::data_value(e, "puntnummer").unwrap_or_default();
        let code = super::data_value(e, "code").unwrap_or_default();
        found.push((u64::from(h), SurveyPoint { name, e: q[0], n: q[1], z: Some(p.location.z / ctx.f), code }));
    }
    if found.is_empty() {
        return Err(crate::lang::tr("geen meetpunten met een code in de tekening of de selectie", "no survey points with a code in the drawing or the selection").into());
    }
    // Handles grow as points are added: this is the measuring order.
    found.sort_by_key(|(h, _)| *h);
    let points: Vec<SurveyPoint> = found.into_iter().map(|(_, p)| p).collect();
    super::api::note("punten", points.len());
    apply_codes(&ctx, &table, &points, &t!("{} punten", "{} points", points.len()), sel.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_files_in_all_flavours() {
        // Dutch: semicolons and decimal commas, X before Y.
        let p = parse("puntnr;X;Y;Z;code\n1;105000,12;425000,34;1,23;put\n2;105010,00;425005,00;-0,50;boom eik\n", Format::Auto).unwrap();
        assert_eq!(p.format, Format::Penzd);
        assert_eq!(p.points[0], SurveyPoint { name: "1".into(), e: 105000.12, n: 425000.34, z: Some(1.23), code: "put".into() });
        assert_eq!(p.points[1].code, "boom eik");
        assert_eq!(p.skipped, 1);
        // Road: PNEZD with commas.
        let p = parse("P1,425000.5,105000.25,2.0,TOP\nP2,425010,105020,2.5,TOP\n", Format::Auto).unwrap();
        assert_eq!(p.format, Format::Pnezd);
        assert_eq!((p.points[0].e, p.points[0].n), (105000.25, 425000.5));
        // Spaces, no names, no code.
        let p = parse("105000 425000 1.5\n105001 425001 1.6\n", Format::Auto).unwrap();
        assert_eq!(p.format, Format::Enzd);
        assert_eq!(p.points[1].name, "2");
        // No height.
        let p = parse("A 105000 425000\n", Format::Penzd).unwrap();
        assert_eq!(p.points[0].z, None);
        // Round trip.
        let again = parse(&write(&p.points, Format::Penzd), Format::Penzd).unwrap();
        assert_eq!(again.points, p.points);
    }

    fn sp(name: &str, e: f64, n: f64, code: &str) -> SurveyPoint {
        SurveyPoint { name: name.into(), e, n, z: Some(1.0), code: code.into() }
    }

    #[test]
    fn codes_are_read_with_line_numbers_and_controls() {
        let t = CodeTable::builtin();
        let c = t.read("LTRB1 B").unwrap();
        assert_eq!((t.codes[c.def].code.as_str(), c.line.as_str(), c.controls.clone()), ("LTRB", "1", vec![Control::Begin]));
        // A code that ends in digits wins over a shorter code with a number.
        let c = t.read("LTR131").unwrap();
        assert_eq!((t.codes[c.def].code.as_str(), c.line.as_str()), ("LTR13", "1"));
        let c = t.read("LTR13").unwrap();
        assert_eq!((t.codes[c.def].code.as_str(), c.line.as_str()), ("LTR13", ""));
        // Glued control code, lower case.
        let c = t.read("ltrb2/cls").unwrap();
        assert_eq!((c.line.as_str(), c.controls.clone()), ("2", vec![Control::Close]));
        // Parameters of a point code.
        let c = t.read("SBM Eik 70").unwrap();
        assert_eq!(t.codes[c.def].kind(), CodeKind::Point);
        assert_eq!(c.params, vec!["Eik".to_string(), "70".to_string()]);
        assert!(t.read("XYZ12").is_none());
        assert!(t.read("").is_none());
    }

    #[test]
    fn the_default_table_resolves_to_nlcs_and_symbols() {
        let t = CodeTable::builtin();
        let mut seen = HashSet::new();
        for c in &t.codes {
            assert!(seen.insert(c.code.to_ascii_uppercase()), "duplicate code {}", c.code);
            if let Some(n) = &c.nlcs {
                let (g, o) = n.split_once(' ').unwrap();
                assert!(crate::nlcs::object_by_name(g, o).is_some(), "{}: NLCS {n} unknown", c.code);
            }
            if let Some(s) = &c.symbool {
                assert!(SYMBOLS.contains(&s.as_str()), "{}: symbol {s}", c.code);
            }
            assert!(c.code.starts_with('L') == (c.kind() == CodeKind::Line), "{}: L-codes are lines", c.code);
        }
    }

    #[test]
    fn figures_follow_begin_end_close_and_line_numbers() {
        let t = CodeTable::builtin();
        let pts = vec![
            sp("1", 0.0, 0.0, "LTRB1 B"),
            sp("2", 10.0, 0.0, "LTRB2 B"),
            sp("3", 0.0, 10.0, "LTRB1"),
            sp("4", 10.0, 10.0, "LTRB2"),
            sp("5", 0.0, 20.0, "LTRB1 E"),
            sp("6", 5.0, 5.0, "SBM Eik 70"),
            // A new string of line 1 after its end.
            sp("7", 50.0, 0.0, "LTRB1"),
            sp("8", 60.0, 0.0, "LTRB1"),
            // A closed building outline.
            sp("9", 100.0, 0.0, "LBGH B"),
            sp("10", 110.0, 0.0, "LBGH"),
            sp("11", 110.0, 10.0, "LBGH CLS"),
            sp("12", 1.0, 1.0, "ONBEKEND 3"),
            sp("13", 1.0, 1.0, "SDMY"),
            sp("14", 200.0, 0.0, "LHK"),
        ];
        let lw = linework(&t, &pts);
        assert_eq!(lw.figures.len(), 4, "{:?}", lw.figures.iter().map(|f| &f.names).collect::<Vec<_>>());
        assert_eq!(lw.figures[0].names, vec!["1", "3", "5"]);
        assert_eq!(lw.figures[1].names, vec!["2", "4"]);
        assert_eq!(lw.figures[2].names, vec!["7", "8"]);
        assert!(lw.figures[3].closed && lw.figures[3].names.len() == 3);
        assert_eq!(lw.symbols.len(), 1);
        assert_eq!(lw.unknown.get("ONBEKEND"), Some(&1));
        assert_eq!(lw.single, 1, "LHK with one point is left out");
        // B on an open string starts a new one.
        let lw = linework(&t, &[sp("1", 0.0, 0.0, "LKAS"), sp("2", 1.0, 0.0, "LKAS"), sp("3", 5.0, 0.0, "LKAS B"), sp("4", 6.0, 0.0, "LKAS")]);
        assert_eq!(lw.figures.len(), 2);
    }

    #[test]
    fn arcs_pass_through_their_points() {
        // Three points on a circle of radius 10 around the origin.
        let (a, b, c) = ([10.0, 0.0, 0.0], [0.0, 10.0, 1.0], [-10.0, 0.0, 2.0]);
        let pts = arc3(a, b, c);
        assert!(pts.contains(&b) && pts.last() == Some(&c));
        for p in &pts {
            assert!((p[0].hypot(p[1]) - 10.0).abs() < 1e-9, "{p:?}");
            assert!(p[1] >= -1e-9, "the arc runs over the top: {p:?}");
        }
        // Straight when the three lie on a line.
        assert_eq!(arc3([0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]).len(), 2);
        // In a figure: BC makes the next two points an arc; EC ends a longer one.
        let t = CodeTable::builtin();
        let ring = |code: &str, k: usize| {
            let a = k as f64 * std::f64::consts::FRAC_PI_4;
            sp(&k.to_string(), 10.0 * a.cos(), 10.0 * a.sin(), code)
        };
        let lw = linework(&t, &[sp("0", 20.0, 0.0, "LKAS"), ring("LKAS BC", 0), ring("LKAS", 1), ring("LKAS", 2), sp("9", -5.0, 20.0, "LKAS")]);
        let p = lw.figures[0].points();
        assert!(p.len() > 10);
        assert!(p.iter().filter(|q| q[1] > 0.0 && q[1] < 10.0 && q[0] > 0.0).all(|q| (q[0].hypot(q[1]) - 10.0).abs() < 1e-9));
        let lw = linework(&t, &[ring("LKAS BC", 0), ring("LKAS", 1), ring("LKAS", 2), ring("LKAS", 3), ring("LKAS EC", 4)]);
        let p = lw.figures[0].points();
        assert!(p.iter().all(|q| (q[0].hypot(q[1]) - 10.0).abs() < 1e-9));
        assert!((p.last().unwrap()[0] + 10.0).abs() < 1e-9);
        // OC: a single point on an arc between its neighbours.
        let lw = linework(&t, &[ring("LKAS", 0), ring("LKAS OC", 2), ring("LKAS", 4)]);
        assert!(lw.figures[0].points().iter().all(|q| (q[0].hypot(q[1]) - 10.0).abs() < 1e-9));
    }
}
