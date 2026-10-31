//! Commands with arguments, for scripts and agents (REST, MCP). Every CIV_
//! command takes `KEY=value` arguments next to its ribbon behaviour; with
//! `ID=<token>` the outcome is written as JSON to `<config>/api/<token>.json`
//! (written whole, then renamed, so a reader never sees half a file).
//!
//! Coordinates in arguments are project coordinates: metres in the project
//! CRS (RD in the Netherlands), "x,y" or "x,y,z"; lists are separated by ";".

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;

use ocs_plugin_api::host::codec::Handle;
use serde_json::{Map, Value};

use super::landmeten::Format;
use super::CivielOptions;
use crate::gis::crs::XY;

pub struct Args {
    /// Words without "=": file paths, coordinates.
    pub words: Vec<String>,
    kv: HashMap<String, String>,
}

/// Split on whitespace, keeping "quoted parts" together.
fn split(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn num(t: &str) -> Result<f64, String> {
    let t = t.trim();
    // "1.234,5" is not expected here: a comma is a decimal comma only
    // where it cannot separate coordinates (single numbers).
    t.parse::<f64>().or_else(|_| t.replace(',', ".").parse::<f64>()).map_err(|_| format!("geen getal: '{t}'"))
}

fn point(t: &str) -> Result<Vec<f64>, String> {
    let parts: Vec<&str> = t.split(',').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return Err(format!("geen punt x,y of x,y,z: '{t}'"));
    }
    parts.iter().map(|p| p.trim().parse::<f64>().map_err(|_| format!("geen punt x,y: '{t}'"))).collect()
}

/// Settings that arguments may change, with the field they set.
pub const SETTINGS: &[(&str, &str)] = &[
    ("SCHAAL", "tekenschaal, 500 voor 1:500"),
    ("STATUS", "tekenstatus B, N, T of V"),
    ("FORMAAT", "puntbestand: AUTO, PNEZD, PENZD, NEZD, ENZD"),
    ("LABELS", "puntlabels ja/nee"),
    ("MAXRIBBE", "TIN: langste driehoekszijde (m)"),
    ("AFSTAND", "TIN uit AHN: puntafstand (m)"),
    ("INTERVAL", "hoogtelijnen: interval (m)"),
    ("HOOFD", "hoogtelijnen: elke n-de is een hoofdlijn"),
    ("STREEP", "as: streepjes om de (m)"),
    ("STATIONERING", "as: stationering om de (m)"),
    ("STAP", "profielen: monsterafstand (m)"),
    ("OVERHOGING", "lengteprofiel: overhoging"),
    ("BOOG", "ontwerpprofiel: lengte verticale bogen (m)"),
    ("OM", "dwarsprofielen: om de (m)"),
    ("LINKS", "dwarsprofielen: breedte links (m)"),
    ("RECHTS", "dwarsprofielen: breedte rechts (m)"),
    ("DOVERHOGING", "dwarsprofielen: overhoging"),
    ("PERRIJ", "dwarsprofielen per rij"),
    ("VERSCHUIVING", "as verschuiven: afstanden, links < 0, bijv. -3.5;3.5"),
    ("BANDEN", "hoogteanalyse: aantal banden"),
    ("CEL", "grondverzet: rastercel (m)"),
    ("PEIL", "grondverzet: peil (m)"),
    ("SJABLOON", "corridor: weg, fietspad, snelweg of dijk"),
    ("FREQUENTIE", "corridor: dwarsprofiel om de (m)"),
];

impl Args {
    pub fn parse(s: &str) -> Args {
        let mut words = Vec::new();
        let mut kv = HashMap::new();
        for w in split(s) {
            match w.split_once('=') {
                Some((k, v)) if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                    kv.insert(k.to_ascii_uppercase(), v.to_string());
                }
                _ => words.push(w),
            }
        }
        Args { words, kv }
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty() && self.kv.is_empty()
    }

    pub fn str(&self, k: &str) -> Option<&str> {
        self.kv.get(k).map(String::as_str)
    }

    pub fn f64(&self, k: &str) -> Result<Option<f64>, String> {
        self.str(k).map(num).transpose().map_err(|e| format!("{k}: {e}"))
    }

    pub fn list(&self, k: &str) -> Result<Option<Vec<f64>>, String> {
        self.str(k).map(|v| v.split(';').filter(|t| !t.trim().is_empty()).map(num).collect()).transpose().map_err(|e| format!("{k}: {e}"))
    }

    /// "x,y;x,y;…" (project coordinates).
    pub fn points(&self, k: &str) -> Result<Option<Vec<XY>>, String> {
        self.str(k)
            .map(|v| v.split(';').filter(|t| !t.trim().is_empty()).map(|t| point(t).map(|p| [p[0], p[1]])).collect())
            .transpose()
            .map_err(|e| format!("{k}: {e}"))
    }

    /// "s:z;s:z;…".
    pub fn pairs(&self, k: &str) -> Result<Option<Vec<(f64, f64)>>, String> {
        self.str(k)
            .map(|v| {
                v.split(';')
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| {
                        let (a, b) = t.split_once(':').ok_or_else(|| format!("verwacht station:hoogte, kreeg '{t}'"))?;
                        Ok((num(a)?, num(b)?))
                    })
                    .collect()
            })
            .transpose()
            .map_err(|e: String| format!("{k}: {e}"))
    }

    /// A handle as the REST API writes it: hexadecimal, "E9E".
    pub fn handle(&self, k: &str) -> Result<Option<Handle>, String> {
        self.str(k).map(parse_handle).transpose().map_err(|e| format!("{k}: {e}"))
    }

    /// Coordinates given as loose words: "x,y" or "x,y,z".
    pub fn coords(&self) -> Result<Vec<Vec<f64>>, String> {
        self.words.iter().filter(|w| w.contains(',')).map(|w| point(w)).collect()
    }

    /// The arguments without one key: for a command whose own key has the
    /// name of a setting (CIV_RIOOL_EXPORT FORMAAT=GWSW).
    pub fn without(&self, k: &str) -> Args {
        Args { words: self.words.clone(), kv: self.kv.iter().filter(|(key, _)| key.as_str() != k).map(|(a, b)| (a.clone(), b.clone())).collect() }
    }

    /// A safe result name.
    pub fn id(&self) -> Option<String> {
        self.str("ID").filter(|v| !v.is_empty() && v.len() <= 64 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).map(str::to_string)
    }

    /// The settings with this command's overrides; unknown keys are an
    /// error that lists the keys this command knows.
    pub fn apply(&self, base: &CivielOptions, own: &[&str]) -> Result<CivielOptions, String> {
        let mut o = base.clone();
        for (k, v) in &self.kv {
            let k = k.as_str();
            let f = || num(v).map_err(|e| format!("{k}: {e}"));
            match k {
                "ID" => {}
                "SCHAAL" => o.nlcs.scale = f()?.max(1.0),
                "STATUS" => o.nlcs.status = crate::nlcs::Status::from_code(&v.to_ascii_uppercase()).ok_or("STATUS: B, N, T of V")?,
                "FORMAAT" => {
                    o.format = match v.to_ascii_uppercase().as_str() {
                        "AUTO" => Format::Auto,
                        "PNEZD" | "PNEZ" => Format::Pnezd,
                        "PENZD" | "PENZ" | "PXYZD" => Format::Penzd,
                        "NEZD" | "NEZ" => Format::Nezd,
                        "ENZD" | "ENZ" | "XYZ" => Format::Enzd,
                        _ => return Err("FORMAAT: AUTO, PNEZD, PENZD, NEZD of ENZD".into()),
                    }
                }
                "LABELS" => o.point_labels = !matches!(v.to_ascii_lowercase().as_str(), "nee" | "no" | "0" | "false" | "uit"),
                "MAXRIBBE" => o.max_edge = f()?.max(0.0),
                "AFSTAND" => o.ahn_spacing = f()?.max(0.5),
                "INTERVAL" => o.interval = f()?.max(0.01),
                "HOOFD" => o.major_every = f()?.max(1.0) as u32,
                "STREEP" => o.tick = f()?.max(0.5),
                "STATIONERING" => o.label_every = f()?.max(1.0),
                "STAP" => o.step = f()?.max(0.05),
                "OVERHOGING" => o.exaggeration = f()?.max(1.0),
                "BOOG" => o.vertical_curve = f()?.max(0.0),
                "OM" => o.section_every = f()?.max(0.5),
                "LINKS" => o.left = f()?.max(0.5),
                "RECHTS" => o.right = f()?.max(0.5),
                "DOVERHOGING" => o.section_exaggeration = f()?.max(1.0),
                "PERRIJ" => o.per_row = f()?.max(1.0) as usize,
                "VERSCHUIVING" => o.offsets = self.list("VERSCHUIVING")?.unwrap_or_default(),
                "BANDEN" => o.bands = f()?.clamp(2.0, 9.0) as usize,
                "CEL" => o.volume_cell = f()?.max(0.1),
                "PEIL" => o.level = f()?,
                "SJABLOON" => o.assembly = v.to_ascii_lowercase(),
                "FREQUENTIE" => o.corridor_every = f()?.max(0.5),
                k if own.contains(&k) => {}
                k => {
                    let mut known: Vec<&str> = own.to_vec();
                    known.extend(SETTINGS.iter().map(|(k, _)| *k));
                    known.push("ID");
                    return Err(format!("onbekend argument {k}=; bekend: {}", known.join(", ")));
                }
            }
        }
        Ok(o)
    }
}

pub fn parse_handle(v: &str) -> Result<Handle, String> {
    u64::from_str_radix(v.trim().trim_start_matches("0x"), 16).map(Handle::from).map_err(|_| format!("geen handle: '{v}'"))
}

pub fn handle_text(h: Handle) -> String {
    format!("{:X}", u64::from(h))
}

thread_local! {
    static DATA: RefCell<Map<String, Value>> = RefCell::new(Map::new());
}

/// Structured outcome of the running command (one per worker thread).
pub fn note(key: &str, value: impl Into<Value>) {
    DATA.with(|d| {
        d.borrow_mut().insert(key.to_string(), value.into());
    });
}

pub fn take() -> Map<String, Value> {
    DATA.with(|d| std::mem::take(&mut *d.borrow_mut()))
}

pub fn result_dir() -> PathBuf {
    crate::gis::servers::config_dir().join("api")
}

/// Write `<config>/api/<id>.json`: {"ok", "message", "data"}.
pub fn write(id: &str, ok: bool, message: &str, data: Map<String, Value>) {
    let dir = result_dir();
    let _ = std::fs::create_dir_all(&dir);
    let body = serde_json::json!({ "ok": ok, "message": message, "data": Value::Object(data) });
    let tmp = dir.join(format!(".{id}.tmp"));
    if std::fs::write(&tmp, serde_json::to_vec_pretty(&body).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join(format!("{id}.json")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments() {
        let a = Args::parse(r#"AS=E9E pi=0,0;100,0;150,50 R=50 OM=12,5 "C:\Mijn map\punten.csv" 105000,425000,1.5 ID=t-1"#);
        assert_eq!(a.handle("AS").unwrap(), Some(Handle::from(0xE9E)));
        assert_eq!(a.points("PI").unwrap().unwrap(), vec![[0.0, 0.0], [100.0, 0.0], [150.0, 50.0]]);
        assert_eq!(a.f64("OM").unwrap(), Some(12.5));
        assert_eq!(a.words[0], r"C:\Mijn map\punten.csv");
        assert_eq!(a.coords().unwrap(), vec![vec![105000.0, 425000.0, 1.5]]);
        assert_eq!(a.id().as_deref(), Some("t-1"));
        let o = a.apply(&CivielOptions::default(), &["AS", "PI", "R"]).unwrap();
        assert_eq!(o.section_every, 12.5);
        let err = Args::parse("FOO=1").apply(&CivielOptions::default(), &[]).unwrap_err();
        assert!(err.contains("onbekend argument FOO") && err.contains("INTERVAL"));
        assert_eq!(Args::parse("PVI=0:1.5;100:2,5").pairs("PVI").unwrap().unwrap(), vec![(0.0, 1.5), (100.0, 2.5)]);
        assert!(Args::parse("ID=../x").id().is_none());
        assert_eq!(handle_text(Handle::from(0xE9E)), "E9E");
    }
}
