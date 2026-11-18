//! BGT — Basisregistratie Grootschalige Topografie via PDOK's OGC API
//! Features, grouped the way a designer thinks about the base map.

use super::ogcapi;
use super::site::Site;
use super::vector::Feature;

pub const API: &str = "https://api.pdok.nl/lv/bgt/ogc/v1";

pub struct Group {
    pub id: &'static str,
    pub label: &'static str,
    pub label_en: &'static str,
    pub collections: &'static [&'static str],
    pub default_on: bool,
}

pub const GROUPS: &[Group] = &[
    Group { id: "wegen", label: "Wegdelen en ondersteunende wegdelen", label_en: "Road sections and supporting road areas", collections: &["wegdeel", "ondersteunendwegdeel"], default_on: true },
    Group { id: "terrein", label: "Begroeide en onbegroeide terreindelen", label_en: "Vegetated and unvegetated land", collections: &["begroeidterreindeel", "onbegroeidterreindeel"], default_on: true },
    Group { id: "water", label: "Waterdelen", label_en: "Water areas", collections: &["waterdeel", "ondersteunendwaterdeel"], default_on: true },
    Group { id: "gebouwen", label: "Panden en overige bouwwerken", label_en: "Buildings and other structures", collections: &["pand", "overigbouwwerk", "gebouwinstallatie"], default_on: true },
    Group { id: "kunstwerken", label: "Kunstwerken (bruggen, tunnels, duikers, keermuren)", label_en: "Structures (bridges, tunnels, culverts, retaining walls)", collections: &["overbruggingsdeel", "tunneldeel", "kunstwerkdeel_vlak", "kunstwerkdeel_lijn", "kunstwerkdeel_punt"], default_on: true },
    Group { id: "scheidingen", label: "Scheidingen (muren, hekken, kademuren)", label_en: "Barriers (walls, fences, quay walls)", collections: &["scheiding_vlak", "scheiding_lijn", "overigescheiding"], default_on: true },
    Group { id: "spoor", label: "Spoor", label_en: "Railways", collections: &["spoor"], default_on: true },
    Group {
        id: "inrichting",
        label: "Inrichtingselementen (palen, putten, kasten, borden, masten, meubilair)", label_en: "Street assets (posts, chambers, cabinets, signs, masts, furniture)",
        collections: &["weginrichtingselement_vlak", "weginrichtingselement_lijn", "weginrichtingselement_punt", "waterinrichtingselement_lijn", "waterinrichtingselement_punt", "straatmeubilair", "paal", "put", "kast", "bord", "mast", "sensor_punt", "sensor_lijn", "bak", "installatie"],
        default_on: false,
    },
    Group { id: "vegetatie", label: "Vegetatieobjecten (bomen, hagen)", label_en: "Vegetation features (trees, hedges)", collections: &["vegetatieobject_vlak", "vegetatieobject_lijn", "vegetatieobject_punt"], default_on: false },
    Group { id: "labels", label: "Straatnamen en huisnummers", label_en: "Street names and house numbers", collections: &["openbareruimtelabel", "pand_nummeraanduiding"], default_on: true },
    Group { id: "kruinlijnen", label: "Kruinlijnen (taluds)", label_en: "Crest lines (slopes)", collections: &["wegdeel_kruinlijn", "ondersteunendwegdeel_kruinlijn", "begroeidterreindeel_kruinlijn", "onbegroeidterreindeel_kruinlijn"], default_on: false },
];

pub fn is_label(collection: &str) -> bool {
    matches!(collection, "openbareruimtelabel" | "pand_nummeraanduiding")
}

/// The text and rotation (radians, counter-clockwise) of a label feature.
pub fn label(collection: &str, f: &Feature) -> Option<(String, f64)> {
    let key = if collection == "openbareruimtelabel" { "openbareruimtenaam" } else { "tekst" };
    let text = f.props.get(key)?.as_str()?.trim().to_string();
    if text.is_empty() {
        return None;
    }
    let hoek = f.props.get("hoek").and_then(|v| v.as_f64()).unwrap_or(0.0).rem_euclid(360.0);
    Some((text, hoek.to_radians()))
}

/// The collection keeps every version of an object; the current one has no
/// `eind_registratie` (and an object that left the BGT a `termination_date`).
pub const CURRENT: &str = "eind_registratie IS NULL";

pub fn is_current(f: &Feature) -> bool {
    let set = |k: &str| f.props.get(k).is_some_and(|v| !v.is_null());
    !set("eind_registratie") && !set("termination_date")
}

/// Fetch the current BGT objects of the collections in the site extent
/// (always requested in RD).
pub fn fetch(site: &Site, collections: &[&str], progress: &dyn Fn(String)) -> Result<Vec<(String, Vec<Feature>)>, String> {
    let b = if site.crs == "EPSG:28992" {
        site.bbox()
    } else {
        super::site::envelope(site.bbox(), &super::crs::Transformer::new(&site.crs, "EPSG:28992")?)?
    };
    let mut out = Vec::new();
    for c in collections {
        // A server without CQL2 support gets the plain request and the
        // history filtered out here.
        let (mut features, _) = match ogcapi::fetch_items(API, c, b, 28992, Some(CURRENT), 50_000, progress) {
            Ok(r) => r,
            Err(_) => ogcapi::fetch_items(API, c, b, 28992, None, 50_000, progress)?,
        };
        features.retain(is_current);
        out.push((c.to_string(), features));
    }
    Ok(out)
}
