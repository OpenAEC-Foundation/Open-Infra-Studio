//! Regional capabilities are independent of the interface language and drawing units.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Region {
    #[default]
    #[serde(rename = "NL")]
    Netherlands,
    #[serde(rename = "US")]
    UnitedStates,
    #[serde(rename = "international")]
    #[serde(other)]
    International,
}
pub const REGIONS: [Region; 3] = [
    Region::International,
    Region::Netherlands,
    Region::UnitedStates,
];
impl Region {
    pub fn code(self) -> &'static str {
        match self {
            Self::Netherlands => "NL",
            Self::UnitedStates => "US",
            Self::International => "international",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Netherlands => crate::lang::tr("Nederland", "Netherlands"),
            Self::UnitedStates => crate::lang::tr("Verenigde Staten", "United States"),
            Self::International => crate::lang::tr("Internationaal", "International"),
        }
    }
    pub fn countries(self) -> Vec<String> {
        match self {
            Self::International => vec![],
            _ => vec![self.code().into(), "GLOBAL".into()],
        }
    }
    pub fn standard(self) -> &'static str {
        match self {
            Self::Netherlands => "NLCS",
            Self::UnitedStates => crate::lang::tr(
                "NCS / A/E/C / State DOT — projectspecifiek",
                "NCS / A/E/C / State DOT — project-specific",
            ),
            Self::International => crate::lang::tr("Projectspecifiek", "Project-specific"),
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
        Self::Netherlands=>crate::lang::tr("RD New en NAP; PDOK, BAG, BGT, BRK, KLIC, AHN, BRO en IMRO. NLCS-hulpmiddelen zijn beschikbaar.","RD New and NAP; PDOK, BAG, BGT, BRK, KLIC, AHN, BRO and IMRO. NLCS tools are available."),
        Self::UnitedStates=>crate::lang::tr("USGS/3DEP, Census TIGERweb, NRCS-bodemdata en lokale registers. Kies de juiste State Plane- of UTM-zone en het horizontale/verticale datum. Gebruik een metrisch project-CRS; de tekening kan in feet staan. Een Amerikaans CAD-standaardprofiel is nog niet geïmplementeerd.","USGS/3DEP, Census TIGERweb, NRCS soils and local records. Select the correct State Plane or UTM zone and horizontal/vertical datum. Use a metric project CRS; the drawing can use feet. A US CAD-standard profile is not implemented yet."),
        Self::International=>crate::lang::tr("Wereldwijde en eigen WMS/WFS/XYZ-services, OpenStreetMap en open bestandsformaten. Kies project-CRS, eenheden, hoogtereferentie en opleverstandaard expliciet.","Worldwide and custom WMS/WFS/XYZ services, OpenStreetMap and open file formats. Select project CRS, units, vertical reference and delivery standard explicitly."),
    }
    }
    pub fn references(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Netherlands => &[
                ("PDOK", "https://www.pdok.nl/"),
                ("NLCS", "https://www.digigo.nu/standaarden/nlcs/"),
            ],
            Self::UnitedStates => &[
                (
                    "USGS — The National Map",
                    "https://apps.nationalmap.gov/services/",
                ),
                ("Census — TIGERweb", "https://tigerweb.geo.census.gov/"),
                (
                    "USDA NRCS — Soil Data Access",
                    "https://sdmdataaccess.nrcs.usda.gov/",
                ),
                (
                    "NOAA NGS — current reference systems",
                    "https://www.ngs.noaa.gov/TOOLS/spc.html",
                ),
                (
                    "NIST — international and survey foot",
                    "https://www.nist.gov/pml/us-surveyfoot",
                ),
            ],
            Self::International => &[("OGC standards", "https://www.ogc.org/standards/")],
        }
    }
}
pub fn from_settings(v: &Value) -> Region {
    if v.get("region").is_some() {
        return serde_json::from_value(v["region"].clone()).unwrap_or(Region::International);
    }
    if v["netherlands"].as_bool() == Some(false) {
        Region::International
    } else {
        Region::Netherlands
    }
}
fn path() -> std::path::PathBuf {
    crate::gis::servers::config_dir().join("settings.json")
}
pub fn configured() -> Region {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .map(|v| from_settings(&v))
        .unwrap_or_default()
}
pub fn current() -> Region {
    static R: OnceLock<Region> = OnceLock::new();
    *R.get_or_init(configured)
}
pub fn save(region: Region) -> Result<(), String> {
    let path = path();
    let mut value = match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str::<Value>(&s).map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e.to_string()),
    };
    if !value.is_object() {
        return Err("Settings must be a JSON object".into());
    }
    value["region"] = serde_json::to_value(region).map_err(|e| e.to_string())?;
    value["netherlands"] = (region == Region::Netherlands).into();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        path,
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn region_is_independent_of_language_and_legacy_switch_migrates() {
        assert_eq!(
            from_settings(&json!({"language":"nl","region":"US","netherlands":true})),
            Region::UnitedStates
        );
        assert_eq!(
            from_settings(&json!({"language":"en","netherlands":true})),
            Region::Netherlands
        );
        assert_eq!(
            from_settings(&json!({"netherlands":false})),
            Region::International
        );
        assert_eq!(
            from_settings(&json!({"region":"unknown"})),
            Region::International
        );
        assert_eq!(from_settings(&json!({})), Region::Netherlands);
    }
    #[test]
    fn regional_scope_includes_global_sources_without_other_countries() {
        assert_eq!(Region::UnitedStates.countries(), vec!["US", "GLOBAL"]);
        assert!(Region::International.countries().is_empty());
    }
}
