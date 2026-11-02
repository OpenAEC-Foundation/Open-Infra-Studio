//! KLIC — gebiedsinformatielevering (WIBON, IMKL 2.0) from Kadaster, read
//! with OpenAEC's `klic2geo` (crates-warehouse).

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

use super::vector::{parse_geojson, Feature};

pub struct Levering {
    pub meldnummer: String,
    pub features: Vec<Feature>,
    /// Elements per theme.
    pub themes: BTreeMap<String, usize>,
    /// Network operators (netbeheerders) in the delivery.
    pub operators: Vec<String>,
}

/// Read a delivery: the Kadaster zip, or an extracted `GI_gebiedsinformatielevering_*.xml`.
pub fn read(path: &Path) -> Result<Levering, String> {
    let lev = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("xml") | Some("gml") => {
            let xml = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            let meld = path.file_stem().and_then(|s| s.to_str()).unwrap_or("").rsplit('_').nth(1).unwrap_or("").to_string();
            klic2geo::parse_gi_xml(&xml, &meld)?
        }
        _ => klic2geo::convert_zip(path)?,
    };
    let fc = serde_json::json!({ "type": "FeatureCollection", "features": lev.features });
    let features = parse_geojson(&fc);
    let mut themes = BTreeMap::new();
    let mut operators: Vec<String> = Vec::new();
    for f in &features {
        *themes.entry(thema(f).to_string()).or_insert(0) += 1;
        if let Some(op) = f.props.get("netbeheerder").and_then(Value::as_str) {
            if !operators.iter().any(|o| o == op) {
                operators.push(op.to_string());
            }
        }
    }
    operators.sort();
    Ok(Levering { meldnummer: lev.meldnummer, features, themes, operators })
}

pub fn thema(f: &Feature) -> &str {
    f.props.get("thema").and_then(Value::as_str).unwrap_or("onbekend")
}

pub fn kind(f: &Feature) -> &str {
    f.props.get("type").and_then(Value::as_str).unwrap_or("")
}

/// Short description for a label: operator, material, diameter.
pub fn describe(f: &Feature) -> String {
    let mut parts = Vec::new();
    for key in ["netbeheerder", "materiaal", "diameter", "appurtenanceType"] {
        if let Some(s) = attr(f, key).filter(|s| !s.is_empty()) {
            parts.push(s.to_string());
        }
    }
    parts.join(" · ")
}

/// An attribute on the feature itself or in its IMKL source block.
pub fn attr<'a>(f: &'a Feature, key: &str) -> Option<&'a str> {
    f.props.get(key).and_then(Value::as_str).or_else(|| f.props.get("bron").and_then(|b| b.get(key)).and_then(Value::as_str))
}

/// Every string attribute as (key, value), for the object data.
pub fn attributes(f: &Feature) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut add = |k: &str, v: &Value| {
        let s = match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            _ => return,
        };
        if !s.is_empty() {
            out.push((k.to_string(), s));
        }
    };
    for (k, v) in &f.props {
        if k == "bron" {
            if let Some(b) = v.as_object() {
                for (bk, bv) in b {
                    add(bk, bv);
                }
            }
        } else {
            add(k, v);
        }
    }
    out
}

/// A short label for a cable or pipe, as on a KLIC map: theme, operator,
/// diameter and material ("W Evides 174 mm GIJ").
pub fn label(f: &Feature) -> String {
    let abbr = match thema(f) {
        "laagspanning" => "LS",
        "middenspanning" => "MS",
        "hoogspanning" => "HS",
        "landelijkHoogspanningsnet" => "HS",
        "datatransport" => "D",
        "gasLageDruk" => "G-LD",
        "gasHogeDruk" => "G-HD",
        "water" => "W",
        "warmte" => "WA",
        "rioolVrijverval" => "RI",
        "rioolOnderOverOfOnderdruk" => "RI-P",
        "petrochemie" | "buisleidingGevaarlijkeInhoud" => "CH",
        _ => "",
    };
    let operator = attr(f, "netbeheerder").map(short_operator).unwrap_or_default();
    let mut parts: Vec<String> = [abbr.to_string(), operator].into_iter().filter(|s| !s.is_empty()).collect();
    if let Some(d) = attr(f, "diameter").filter(|d| !d.is_empty()) {
        parts.push(d.to_string());
    }
    if let Some(m) = attr(f, "materiaal").filter(|m| !m.is_empty() && *m != "onbekend") {
        parts.push(material(m).to_string());
    }
    parts.join(" ")
}

fn short_operator(name: &str) -> String {
    let skip = ["B.V.", "BV", "N.V.", "NV", "Netbeheer", "Netherlands", "Nederland", "Infra", "Gebroeders"];
    name.split(|c: char| c.is_whitespace() || c == '(')
        .map(|w| w.trim_matches(')'))
        .find(|w| !w.is_empty() && !skip.contains(w))
        .unwrap_or(name)
        .to_string()
}

/// IMKL material code → the usual Dutch abbreviation.
fn material(code: &str) -> &str {
    match code {
        "castIron" | "grijsGietijzer" => "GIJ",
        "ductileIron" => "NGIJ",
        "steel" => "ST",
        "asbestos-cement" | "asbestosCement" => "AC",
        "concrete" | "reinforcedConcrete" => "BT",
        "polyvinylChloride" | "PVC" | "pvc" => "PVC",
        "polyethylene" | "PE" | "polyetheen" => "PE",
        "highDensityPolyethylene" | "HPE" => "HPE",
        "masonry" => "MW",
        "copper" => "CU",
        "stoneware" => "GRES",
        other => other,
    }
}

/// Human-readable theme names.
pub fn theme_label(thema: &str) -> &str {
    match thema {
        "laagspanning" => "Laagspanning",
        "middenspanning" => "Middenspanning",
        "hoogspanning" => "Hoogspanning",
        "landelijkHoogspanningsnet" => "Landelijk hoogspanningsnet",
        "datatransport" => "Datatransport",
        "gasLageDruk" => "Gas lage druk",
        "gasHogeDruk" => "Gas hoge druk",
        "water" => "Water",
        "warmte" => "Warmte",
        "rioolVrijverval" => "Riool vrijverval",
        "rioolOnderOverOfOnderdruk" => "Riool onder/overdruk",
        "petrochemie" => "(Petro)chemie",
        "buisleidingGevaarlijkeInhoud" => "Buisleiding gevaarlijke inhoud",
        "graafpolygoon" => "Graafpolygoon",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML_NAME: &str = "GI_gebiedsinformatielevering_SYNTHETIC_1_V2.xml";
    const XML: &str = include_str!("../../tests/fixtures/klic/GI_gebiedsinformatielevering_SYNTHETIC_1_V2.xml");

    #[test]
    fn fixed_klic_xml_and_zip_preserve_the_same_delivery_and_geometry() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/klic");
        for path in [dir.join(XML_NAME), dir.join("delivery.zip")] {
            let delivery = read(&path).unwrap();
            assert_eq!(delivery.meldnummer, "SYNTHETIC", "{}", path.display());
            assert_eq!(delivery.features.len(), 1);
            assert_eq!(delivery.features[0].lines[0].0, vec![[100.0, 200.0], [110.0, 210.0]]);
            assert_eq!(delivery.themes.get("water"), Some(&1));
        }
    }

    #[test]
    fn klic_returns_errors_for_empty_broken_and_missing_files() {
        let dir = std::env::temp_dir().join(format!("oi-klic-invalid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (index, text) in ["", "not xml", "<", "<Other/>"].iter().enumerate() {
            let path = dir.join(format!("invalid-{index}.xml"));
            std::fs::write(&path, text).unwrap();
            assert!(read(&path).is_err(), "{text}");
        }
        let zip = dir.join("broken.zip");
        std::fs::write(&zip, b"not a zip").unwrap();
        assert!(read(&zip).is_err());
        assert!(read(&dir.join("missing.xml")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn klic_dependency_panics_become_errors_at_the_file_boundary() {
        let dir = std::env::temp_dir().join(format!("oi-klic-unicode-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(XML_NAME);
        std::fs::write(&path, XML.replace("nl.imkl-KL9999.synthetic", "é-KL9999.synthetic")).unwrap();
        let result = read(&path);
        assert!(result.is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The Dordrecht sample delivery from GIS2BIM (Lange Geldersekade 2).
    #[test]
    #[ignore]
    fn reads_gis2bim_sample() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../GIS2BIM/supportfiles/Sandbox/KLIC/Levering_21O072584_1.zip");
        let lev = read(&p).expect("read");
        println!("meldnummer {} features {} themes {:?} operators {:?}", lev.meldnummer, lev.features.len(), lev.themes, lev.operators);
        let mut kinds = BTreeMap::new();
        for f in &lev.features {
            *kinds.entry(format!("{} / {} / l{} p{}", thema(f), kind(f), f.lines.len(), f.points.len())).or_insert(0) += 1;
        }
        for (k, n) in kinds {
            println!("{n:5}  {k}");
        }
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for f in &lev.features {
            for p in f.lines.iter().flat_map(|(l, _)| l.iter()).chain(f.points.iter()) {
                b = [b[0].min(p[0]), b[1].min(p[1]), b[2].max(p[0]), b[3].max(p[1])];
            }
        }
        println!("extent RD {b:?}");
        let mats: std::collections::BTreeSet<String> = lev.features.iter().filter_map(|f| attr(f, "materiaal").map(str::to_string)).collect();
        println!("materials {mats:?}");
    }
}
