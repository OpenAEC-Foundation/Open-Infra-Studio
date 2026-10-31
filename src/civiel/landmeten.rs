//! Survey points (Road "GeoPoints"): point files in, points out, points
//! placed with their ground height.

use std::collections::HashSet;

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
    Ok(format!(
        "{n} punten uit {name} ({}{range}){} — {written} elementen{}.",
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
}
