//! KLIC deliveries read in by themselves: `GIS_KLIC_MAP` reads every delivery
//! in a folder (the Kadaster's `Levering_*.zip`, or an extracted folder's
//! `GI_gebiedsinformatielevering_*.xml`) that this drawing has not read yet,
//! and with `VOLGEN=ja` keeps looking (every minute, while Open CAD Studio
//! runs) so a delivery saved there from the e-mail or the Kadaster portal
//! comes into the drawing without a step. Which meldnummers a drawing has
//! read is kept in it (XRecord `OPENINFRA/KLIC_GELEZEN`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use crate::basisdata::{self, KlicOptions};
use crate::bridge;
use crate::gis::klic;
use crate::gis::site::Site;
use crate::lang::tr;

const RECORD: &str = "KLIC_GELEZEN";

/// The one folder being followed; a new `GIS_KLIC_MAP` (or `VOLGEN=nee`) ends the previous.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The candidates in a folder, oldest first: zips, and the delivery XML of
/// extracted deliveries (one level down).
pub fn candidates(dir: &Path) -> Vec<PathBuf> {
    let is_delivery_xml = |p: &Path| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("GI_gebiedsinformatielevering") && n.ends_with(".xml"));
    // The Kadaster's name: Levering_<meldnummer>_<versie>.zip.
    let is_delivery_zip = |p: &Path| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.to_ascii_lowercase().starts_with("levering") && n.to_ascii_lowercase().ends_with(".zip"));
    let mut out: Vec<(SystemTime, PathBuf)> = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    for e in entries.flatten() {
        let p = e.path();
        let time = e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        if p.is_dir() {
            if let Ok(inner) = std::fs::read_dir(&p) {
                out.extend(inner.flatten().map(|f| f.path()).filter(|f| is_delivery_xml(f)).map(|f| (time, f)));
            }
        } else if is_delivery_zip(&p) || is_delivery_xml(&p) {
            out.push((time, p));
        }
    }
    out.sort();
    out.into_iter().map(|(_, p)| p).collect()
}

fn read_done() -> Vec<String> {
    bridge::json_record_read(RECORD).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// One pass: reads what is new. `tried` keeps files that are no delivery
/// (or failed) from being tried again every minute.
fn pass(site: &Site, dir: &Path, tried: &mut HashSet<PathBuf>) -> Result<Vec<String>, String> {
    let mut done = read_done();
    let mut lines = Vec::new();
    for p in candidates(dir) {
        if tried.contains(&p) {
            continue;
        }
        tried.insert(p.clone());
        let lev = match klic::read(&p) {
            Ok(l) => l,
            Err(e) => {
                lines.push(format!("{}: {e}", p.display()));
                continue;
            }
        };
        if lev.meldnummer.is_empty() || done.contains(&lev.meldnummer) {
            continue;
        }
        let r = basisdata::klic_place(site, &lev, &KlicOptions::default())?;
        done.push(lev.meldnummer.clone());
        bridge::json_record_write(RECORD, &serde_json::to_string(&done).map_err(|e| e.to_string())?)?;
        lines.push(format!("{} {}: {r}", tr("KLIC-melding", "KLIC notification"), lev.meldnummer));
    }
    Ok(lines)
}

/// `GIS_KLIC_MAP [MAP=<map>] [VOLGEN=ja|nee]`.
pub fn command(site: Site, dir: Option<PathBuf>, follow: Option<bool>) -> Result<String, String> {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    if follow == Some(false) && dir.is_none() {
        return Ok(tr("KLIC-map wordt niet meer gevolgd.", "KLIC folder no longer followed.").into());
    }
    let dir = dir.unwrap_or_else(default_dir);
    if !dir.is_dir() {
        return Err(format!("{}: {}", tr("geen map", "not a folder"), dir.display()));
    }
    let _ = std::fs::write(crate::gis::servers::config_dir().join("klic-map.txt"), dir.to_string_lossy().as_bytes());
    let mut tried = HashSet::new();
    let lines = pass(&site, &dir, &mut tried)?;
    let now = if lines.is_empty() { tr("geen nieuwe leveringen", "no new deliveries").to_string() } else { lines.join(" | ") };
    if follow != Some(true) {
        return Ok(format!("{} {}: {now}", tr("KLIC-map", "KLIC folder"), dir.display()));
    }
    let target = bridge::target().ok_or("geen tekening actief")?;
    let folder = dir.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(60));
        if GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        // Into the drawing it was started from, even when another is active.
        let r = bridge::with_command_context(target.clone(), Some(site.clone()), None, || pass(&site, &folder, &mut tried));
        match r {
            Ok(lines) if !lines.is_empty() => bridge::info(&format!("[Open Infra] {}", lines.join(" | "))),
            Ok(_) => {}
            Err(e) => {
                // The drawing closed (or cannot be written): stop following.
                bridge::error(&format!("GIS_KLIC_MAP {}: {e}", tr("gestopt", "stopped")));
                return;
            }
        }
    });
    Ok(format!("{} {} ({}): {now}", tr("KLIC-map gevolgd", "KLIC folder followed"), dir.display(), tr("elke minuut, VOLGEN=nee stopt", "every minute, VOLGEN=nee stops")))
}

/// The folder of last time, else the Downloads folder.
pub fn default_dir() -> PathBuf {
    let stored = std::fs::read_to_string(crate::gis::servers::config_dir().join("klic-map.txt")).ok().map(|s| PathBuf::from(s.trim()));
    stored.filter(|p| p.is_dir()).unwrap_or_else(|| {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
        ["Downloads", "Documenten/Downloads"].iter().map(|d| home.join(d)).find(|p| p.is_dir()).unwrap_or(home)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zips_and_extracted_deliveries_oldest_first() {
        let dir = std::env::temp_dir().join(format!("oi-klic-map-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Levering_1")).unwrap();
        std::fs::write(dir.join("Levering_2.zip"), b"").unwrap();
        std::fs::write(dir.join("Levering_1").join("GI_gebiedsinformatielevering_1.xml"), b"").unwrap();
        std::fs::write(dir.join("notities.txt"), b"").unwrap();
        std::fs::write(dir.join("foto's.zip"), b"").unwrap();
        let c = candidates(&dir);
        let names: Vec<String> = c.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.contains(&"Levering_2.zip".into()) && names.contains(&"GI_gebiedsinformatielevering_1.xml".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
