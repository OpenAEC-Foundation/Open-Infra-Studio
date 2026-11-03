//! What a KLIC delivery holds besides the cables and pipes (read by
//! `klic2geo`): the areas with an *eis voorzorgsmaatregel* (a precaution the
//! network operator requires before digging there), the *extra detailinfo*
//! areas and points (a detail drawing or house-connection sketch for that
//! spot) and the attachments (letters, conditions, sketches as PDF or image).
//!
//! Read with a generic pass over the IMKL 2.0 GML, so small differences
//! between IMKL versions do not matter: any feature whose name holds
//! `EisVoorzorgsmaatregel` with a surface is an EV area, any `ExtraDetailinfo`
//! with a location a detail area or point, any `Bijlage` with a
//! `bestandLocatie` an attachment.

use std::io::Read;
use std::path::{Path, PathBuf};

use super::crs::XY;
use crate::lang::tr;

/// The pseudo-themes under which these appear in the theme list.
pub const THEME_EV: &str = "eisVoorzorgsmaatregel";
pub const THEME_DETAIL: &str = "extraDetailinfo";

#[derive(Clone, Debug, Default)]
pub struct Area {
    /// `eis_voorzorgsmaatregel` or `extra_detailinfo`.
    pub kind: &'static str,
    /// The requirement or the detail type, e.g. `aanwezigheidNetbeheerder`, `huisaansluiting`.
    pub value: String,
    /// Theme of the network it belongs to (from `inNetwork`), when known.
    pub thema: String,
    /// The network operator code (KL1031, …) from the identifier.
    pub operator: String,
    /// Polygons in RD: outer ring first, then holes; empty for a point.
    pub polygons: Vec<Vec<Vec<XY>>>,
    pub points: Vec<XY>,
    /// Attachment of a detail, relative to the delivery root (`bronnen/…`).
    pub file: Option<String>,
    pub id: String,
}

#[derive(Clone, Debug, Default)]
pub struct Attachment {
    /// `algemeen`, `eisVoorzorgsmaatregel`, `huisaansluitschets`, …
    pub kind: String,
    /// Relative to the delivery root, e.g. `bronnen/KL1081/….pdf`.
    pub path: String,
    pub operator: String,
}

#[derive(Clone, Debug, Default)]
pub struct Extras {
    pub areas: Vec<Area>,
    pub attachments: Vec<Attachment>,
}

impl Extras {
    pub fn count(&self, kind: &str) -> usize {
        self.areas.iter().filter(|a| a.kind == kind).count()
    }
}

/// The last path segment of a code-list URI (…/BijlageTypeValue/algemeen → algemeen).
fn code(href: &str) -> String {
    href.rsplit('/').next().unwrap_or(href).to_string()
}

/// `KL1081` from `nl.imkl-KL1081.BijlagAlgemeen_…`.
fn operator_of(id: &str) -> String {
    id.split(['-', '.']).find(|p| p.len() >= 4 && p.chars().take(2).all(|c| c.is_ascii_uppercase()) && p[2..].chars().all(|c| c.is_ascii_digit())).unwrap_or("").to_string()
}

fn pos_list(text: &str, dim: usize) -> Vec<XY> {
    let v: Vec<f64> = text.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    v.chunks_exact(dim.max(2)).map(|c| [c[0], c[1]]).collect()
}

fn dimension(n: roxmltree::Node) -> usize {
    n.ancestors()
        .find_map(|a| a.attribute("srsDimension"))
        .and_then(|d| d.parse().ok())
        .unwrap_or(2)
}

/// Polygons below `n`: each `Polygon` or `PolygonPatch` with its rings.
fn polygons(n: roxmltree::Node) -> Vec<Vec<Vec<XY>>> {
    let mut out = Vec::new();
    for p in n.descendants().filter(|d| d.is_element() && matches!(d.tag_name().name(), "Polygon" | "PolygonPatch")) {
        let mut rings = Vec::new();
        for side in ["exterior", "interior"] {
            for r in p.children().filter(|c| c.is_element() && c.tag_name().name() == side) {
                let mut ring = Vec::new();
                for d in r.descendants().filter(|d| d.is_element()) {
                    match d.tag_name().name() {
                        "posList" => ring.extend(pos_list(d.text().unwrap_or(""), dimension(d))),
                        "pos" => ring.extend(pos_list(d.text().unwrap_or(""), dimension(d))),
                        _ => {}
                    }
                }
                if ring.len() >= 3 {
                    rings.push(ring);
                }
            }
        }
        if !rings.is_empty() {
            out.push(rings);
        }
    }
    out
}

fn points(n: roxmltree::Node) -> Vec<XY> {
    n.descendants()
        .filter(|d| d.is_element() && d.tag_name().name() == "Point")
        .flat_map(|p| p.descendants().filter(|d| d.is_element() && d.tag_name().name() == "pos").flat_map(|d| pos_list(d.text().unwrap_or(""), dimension(d))).collect::<Vec<_>>())
        .collect()
}

fn child_text<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name).and_then(|c| c.text()).map(str::trim)
}

fn child_href<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<&'a str> {
    n.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
        .and_then(|c| c.attributes().find(|a| a.name() == "href").map(|a| a.value()))
}

/// Reads the extras from the GI-xml text of a delivery.
pub fn parse(xml: &str) -> Result<Extras, String> {
    let opt = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(xml, opt).map_err(|e| format!("KLIC: {e}"))?;
    let id_of = |n: roxmltree::Node| n.attributes().find(|a| a.name() == "id").map(|a| a.value().to_string()).unwrap_or_default();
    // Network id → theme, for the EV areas that point to their network.
    let mut themes = std::collections::HashMap::new();
    for n in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "Utiliteitsnet") {
        if let Some(t) = child_href(n, "thema") {
            themes.insert(id_of(n), code(t));
        }
    }
    let mut x = Extras::default();
    for n in doc.descendants().filter(|n| n.is_element()) {
        let name = n.tag_name().name();
        if name == "Bijlage" {
            if let Some(path) = child_text(n, "bestandLocatie") {
                x.attachments.push(Attachment { kind: child_href(n, "bijlageType").map(code).unwrap_or_default(), path: path.to_string(), operator: operator_of(&id_of(n)) });
            }
            continue;
        }
        let ev = name.contains("EisVoorzorgsmaatregel") && n.attribute(("http://www.opengis.net/gml/3.2", "id")).or_else(|| n.attributes().find(|a| a.name() == "id").map(|a| a.value())).is_some();
        let detail = name == "ExtraDetailinfo";
        if !ev && !detail {
            continue;
        }
        let id = id_of(n);
        let network = child_href(n, "inNetwork").unwrap_or("");
        let value = n
            .children()
            .filter(|c| c.is_element())
            .find_map(|c| {
                let href = c.attributes().find(|a| a.name() == "href").map(|a| a.value())?;
                (href.contains("EisVoorzorgsmaatregel") || href.contains("ExtraDetailInfoType") || href.contains("ExtraDetailinfoType")).then(|| code(href))
            })
            .unwrap_or_default();
        let area = Area {
            kind: if ev { "eis_voorzorgsmaatregel" } else { "extra_detailinfo" },
            value,
            thema: themes.get(network).cloned().unwrap_or_default(),
            operator: operator_of(&id),
            polygons: polygons(n),
            points: points(n),
            file: child_text(n, "bestandLocatie").map(str::to_string),
            id,
        };
        if !area.polygons.is_empty() || !area.points.is_empty() {
            x.areas.push(area);
        }
        // An EV attachment named in the feature itself.
        if ev {
            if let Some(p) = child_text(n, "bestandLocatie") {
                x.attachments.push(Attachment { kind: THEME_EV.into(), path: p.to_string(), operator: operator_of(&x.areas.last().map(|a| a.id.clone()).unwrap_or_default()) });
            }
        }
    }
    Ok(x)
}

/// The GI-xml of a delivery (zip or extracted folder file) as text.
fn gi_xml(path: &Path) -> Result<String, String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("xml") | Some("gml")) {
        return std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()));
    }
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .find(|n| n.rsplit('/').next().is_some_and(|f| f.starts_with("GI_gebiedsinformatielevering") && f.to_ascii_lowercase().ends_with(".xml")))
        .ok_or_else(|| tr("geen GI_gebiedsinformatielevering-xml in de zip", "no GI_gebiedsinformatielevering xml in the zip").to_string())?;
    let mut s = String::new();
    zip.by_name(&name).map_err(|e| e.to_string())?.read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}

/// Reads the extras of a delivery: the Kadaster zip or the extracted GI-xml.
pub fn read(path: &Path) -> Result<Extras, String> {
    parse(&gi_xml(path)?)
}

/// An attachment as a file on disk: next to an extracted GI-xml it is already
/// there; from a zip it is extracted to `out_dir` (once).
pub fn attachment_file(delivery: &Path, rel: &str, out_dir: &Path) -> Result<PathBuf, String> {
    if rel.contains("..") {
        return Err(format!("{} {rel}", tr("ongeldig bijlagepad", "invalid attachment path")));
    }
    let ext = delivery.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if matches!(ext.as_deref(), Some("xml") | Some("gml")) {
        let p = delivery.parent().unwrap_or(Path::new(".")).join(rel);
        return if p.exists() { Ok(p) } else { Err(format!("{} {}", tr("bijlage niet gevonden:", "attachment not found:"), p.display())) };
    }
    let target = out_dir.join(rel.rsplit('/').next().unwrap_or(rel));
    if target.exists() {
        return Ok(target);
    }
    let file = std::fs::File::open(delivery).map_err(|e| format!("{}: {e}", delivery.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let name = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .find(|n| n.ends_with(rel))
        .ok_or_else(|| format!("{} {rel}", tr("bijlage niet in de zip:", "attachment not in the zip:")))?;
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    zip.by_name(&name).map_err(|e| e.to_string())?.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    Ok(target)
}

/// Opens a file with the system's viewer (PDF reader, image viewer).
pub fn open_file(path: &Path) -> Result<(), String> {
    let status = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", "start", ""]).arg(path).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(path).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(path).spawn()
    };
    status.map(|_| ()).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gml:FeatureCollection xmlns:gml="http://www.opengis.net/gml/3.2" xmlns:imkl="http://www.geostandaarden.nl/imkl/wibon" xmlns:xlink="http://www.w3.org/1999/xlink">
  <gml:featureMember>
    <imkl:Utiliteitsnet gml:id="nl.imkl-KL1031.UNET_GAS_HD">
      <imkl:thema xlink:href="http://definities.geostandaarden.nl/imkl2015/id/waarde/Thema/gasHogeDruk"/>
    </imkl:Utiliteitsnet>
  </gml:featureMember>
  <gml:featureMember>
    <imkl:AanduidingEisVoorzorgsmaatregel gml:id="nl.imkl-KL1031.EV_1">
      <imkl:inNetwork xlink:href="nl.imkl-KL1031.UNET_GAS_HD"/>
      <imkl:eisVoorzorgsmaatregel xlink:href="http://definities.geostandaarden.nl/imkl2015/id/waarde/EisVoorzorgsmaatregelValue/aanwezigheidNetbeheerder"/>
      <imkl:geometrie>
        <gml:Polygon gml:id="EV_1_G" srsName="urn:ogc:def:crs:EPSG::28992">
          <gml:exterior><gml:LinearRing><gml:posList>100 400 140 400 140 440 100 440 100 400</gml:posList></gml:LinearRing></gml:exterior>
          <gml:interior><gml:LinearRing><gml:posList>110 410 120 410 120 420 110 410</gml:posList></gml:LinearRing></gml:interior>
        </gml:Polygon>
      </imkl:geometrie>
    </imkl:AanduidingEisVoorzorgsmaatregel>
  </gml:featureMember>
  <gml:featureMember>
    <imkl:ExtraDetailinfo gml:id="nl.imkl-KL1104.EXDI_2">
      <imkl:extraInfoType xlink:href="http://definities.geostandaarden.nl/imkl2015/id/waarde/ExtraDetailInfoTypeValue/huisaansluiting"/>
      <imkl:bestandLocatie>bronnen/KL1104/schets.pdf</imkl:bestandLocatie>
      <imkl:ligging><gml:Point gml:id="P2" srsDimension="3"><gml:pos>105 405 -1.2</gml:pos></gml:Point></imkl:ligging>
    </imkl:ExtraDetailinfo>
  </gml:featureMember>
  <gml:featureMember>
    <imkl:Bijlage gml:id="nl.imkl-KL1081.BijlagAlgemeen_1">
      <imkl:bijlageType xlink:href="http://definities.geostandaarden.nl/imkl2015/id/waarde/BijlageTypeValue/algemeen"/>
      <imkl:bestandLocatie>bronnen/KL1081/brief.pdf</imkl:bestandLocatie>
    </imkl:Bijlage>
  </gml:featureMember>
</gml:FeatureCollection>"#;

    #[test]
    fn ev_areas_details_and_attachments() {
        let x = parse(XML).unwrap();
        assert_eq!((x.count("eis_voorzorgsmaatregel"), x.count("extra_detailinfo"), x.attachments.len()), (1, 1, 1));
        let ev = &x.areas[0];
        assert_eq!((ev.value.as_str(), ev.thema.as_str(), ev.operator.as_str()), ("aanwezigheidNetbeheerder", "gasHogeDruk", "KL1031"));
        assert_eq!(ev.polygons.len(), 1);
        assert_eq!(ev.polygons[0].len(), 2, "outer ring and one hole");
        assert_eq!(ev.polygons[0][0][1], [140.0, 400.0]);
        let d = &x.areas[1];
        assert_eq!((d.value.as_str(), d.file.as_deref(), d.points.clone()), ("huisaansluiting", Some("bronnen/KL1104/schets.pdf"), vec![[105.0, 405.0]]));
        assert_eq!((x.attachments[0].kind.as_str(), x.attachments[0].operator.as_str()), ("algemeen", "KL1081"));
    }

    #[test]
    fn the_gis2bim_sample_delivery() {
        let p = Path::new("/home/aia10/Documenten/GIS2BIM/supportfiles/Sandbox/KLIC/Levering_21O072584_1.zip");
        if !p.exists() {
            return; // only on the development machine
        }
        let x = read(p).unwrap();
        assert_eq!((x.attachments.len(), x.count("extra_detailinfo")), (14, 4));
        let dir = std::env::temp_dir().join(format!("oi-klic-bijlage-{}", std::process::id()));
        let f = attachment_file(p, &x.attachments[0].path, &dir).unwrap();
        assert!(std::fs::read(&f).unwrap().starts_with(b"%PDF"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn attachment_paths_stay_inside_the_delivery() {
        assert!(attachment_file(Path::new("x.zip"), "../../etc/passwd", Path::new("/tmp")).is_err());
    }
}
