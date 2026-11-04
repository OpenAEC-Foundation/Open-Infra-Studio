//! GWSW-HydX: the exchange format of Stichting RIONED for hydraulic sewer
//! models (InfoWorks ICM, 3Di, SOBEK, …), a folder of CSV files with `;`
//! and decimal points. The designed networks give Knooppunt (manholes),
//! Verbinding (pipes), Profiel (pipe profiles) and Meta; the other files of
//! the set (Kunstwerk, Oppervlak, Debiet, Verloop, Nwrw, ItObject) are
//! written with their header only, as importers expect all ten.
//!
//! Columns and codes as in the GWSW-HydX test set of `nens/hydxlib`
//! (Werkgroep GWSW-Hyd, GWSW-Apps): manholes `INS` with `RND`/`RHK`, pipes
//! `GSL` (infiltration pipes `ITR`), flow direction `OPN`, sewage type
//! `DWA`/`HWA`/`GMD`, existing objects `ACT`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::riool::{Materiaal, Net, Stelsel};
use crate::gis::crs::XY;

pub const FILES: [&str; 10] = ["Knooppunt", "Verbinding", "Profiel", "Kunstwerk", "Oppervlak", "Debiet", "Verloop", "Nwrw", "ItObject", "Meta"];

pub const KNOOPPUNT: &str = "UNI_IDE;RST_IDE;PUT_IDE;KNP_XCO;KNP_YCO;CMP_IDE;MVD_NIV;MVD_SCH;WOS_OPP;KNP_MAT;KNP_VRM;KNP_BOK;KNP_BRE;KNP_LEN;KNP_TYP;INI_NIV;STA_OBJ;AAN_MVD;ITO_IDE;ALG_TOE";
pub const VERBINDING: &str = "UNI_IDE;KN1_IDE;KN2_IDE;VRB_TYP;LEI_IDE;BOB_KN1;BOB_KN2;STR_RCH;VRB_LEN;INZ_TYP;ITO_IDE;PRO_IDE;STA_OBJ;AAN_BB1;AAN_BB2;AAN_PRO;INI_NIV;ALG_TOE";
pub const PROFIEL: &str = "PRO_IDE;PRO_MAT;PRO_VRM;PRO_BRE;PRO_HGT;TAB_BRE;TAB_HGT;ALG_TOE";
const KUNSTWERK: &str = "UNI_IDE;KWK_TYP;BWS_NIV;PRO_BOK;DRL_COE;DRL_CAP;OVS_BRE;OVS_NIV;OVS_VOH;OVS_COE;PMP_CAP;PMP_AN1;PMP_AF1;PMP_AN2;PMP_AF2;QDH_NIV;QDH_DEB;AAN_OVN;AAN_OVB;AAN_CAP;AAN_ANS;AAN_AFS;ALG_TOE";
const OPPERVLAK: &str = "UNI_IDE;NSL_STA;AFV_DEF;AFV_IDE;AFV_OPP;ALG_TOE";
const DEBIET: &str = "UNI_IDE;DEB_TYP;VER_IDE;AVV_ENH;AFV_OPP;ALG_TOE";
const VERLOOP: &str = "VER_IDE;VER_TYP;VER_DAG;VER_VOL;U00_DAG;U01_DAG;U02_DAG;U03_DAG;U04_DAG;U05_DAG;U06_DAG;U07_DAG;U08_DAG;U09_DAG;U10_DAG;U11_DAG;U12_DAG;U13_DAG;U14_DAG;U15_DAG;U16_DAG;U17_DAG;U18_DAG;U19_DAG;U20_DAG;U21_DAG;U22_DAG;U23_DAG;ALG_TOE";
const NWRW: &str = "AFV_IDE;AFV_BRG;AFV_IFX;AFV_IFN;AFV_IFA;AFV_IFH;AFV_AFS;AFV_LEN;AFV_HEL;AFV_RUW;ALG_TOE";
const ITOBJECT: &str = "ITO_IDE;KWA_RIO;KWA_CUN;KWA_GRW;STD_GRW;ALG_TOE";
const META: &str = "ALG_ATL;ALG_VRS;ALG_DAT;ALG_OPD;ALG_UIT;ALG_OMS;ALG_EXP;ALG_TOE";

fn material(m: Materiaal) -> &'static str {
    match m {
        Materiaal::Beton => "BET",
        Materiaal::Pvc => "PVC",
        Materiaal::Pp => "PP",
        Materiaal::Pe => "PE",
        Materiaal::Gres => "GRE",
    }
}

fn sewage(s: Stelsel) -> &'static str {
    match s {
        Stelsel::Dwa => "DWA",
        Stelsel::Hwa | Stelsel::Itr => "HWA",
        Stelsel::Gwa => "GMD",
    }
}

/// A field without the separator or line breaks.
fn field(s: &str) -> String {
    s.replace([';', '\n', '\r'], " ")
}

fn n(v: f64, d: usize) -> String {
    format!("{v:.d$}")
}

/// The ten HydX tables for `nets` (coordinates through `rd`), keyed by file name without `.csv`.
pub fn tables(nets: &[&Net], rd: &dyn Fn(XY) -> Result<XY, String>, date: &str, description: &str) -> Result<BTreeMap<&'static str, String>, String> {
    let mut knp = format!("{KNOOPPUNT}\n");
    let mut vrb = format!("{VERBINDING}\n");
    let mut profiles: BTreeMap<String, String> = BTreeMap::new();
    // Manhole names must be unique over the file: prefix the network when two share one.
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for net in nets {
        for p in &net.putten {
            *count.entry(p.naam.as_str()).or_insert(0) += 1;
        }
    }
    let id = |net: &Net, i: usize| -> String {
        let name = &net.putten[i].naam;
        if count.get(name.as_str()).copied().unwrap_or(0) > 1 { field(&format!("{}_{name}", net.naam)) } else { field(name) }
    };
    for net in nets {
        for (i, p) in net.putten.iter().enumerate() {
            let q = rd(p.xy)?;
            let rect = p.maat[1] > 0.0;
            let bottom = net.bob_laagst(i);
            knp.push_str(&format!(
                "{};{};{};{};{};;{};RES;;BET;{};{};{:.0};{:.0};INS;;{};;;{}\n",
                id(net, i),
                field(&net.naam),
                field(&p.naam),
                n(q[0], 2),
                n(q[1], 2),
                p.maaiveld.map(|v| n(v, 2)).unwrap_or_default(),
                if rect { "RHK" } else { "RND" },
                bottom.map(|v| n(v, 2)).unwrap_or_default(),
                p.maat[0],
                if rect { p.maat[1] } else { p.maat[0] },
                if p.bestaand { "ACT" } else { "" },
                field(&format!("{} {}", p.stelsel.code(), p.maat_text())),
            ));
        }
        for (i, l) in net.leidingen.iter().enumerate() {
            let pro = format!("{}{:.0}", material(l.materiaal), l.dn);
            profiles.entry(pro.clone()).or_insert_with(|| {
                format!("{pro};{};RND;{:.0};;;;{}\n", material(l.materiaal), l.materiaal.binnen(l.dn), field(&format!("{} DN {:.0}", l.materiaal.label(), l.dn)))
            });
            vrb.push_str(&format!(
                "{};{};{};{};{};{};{};OPN;{};{};;{};{};;;;;{}\n",
                field(&net.naam_leiding(i)),
                id(net, l.van),
                id(net, l.naar),
                if l.stelsel == Stelsel::Itr { "ITR" } else { "GSL" },
                field(&net.naam_leiding(i)),
                n(l.bob_van, 2),
                n(l.bob_naar, 2),
                n(net.lengte(i), 2),
                sewage(l.stelsel),
                pro,
                if l.bestaand { "ACT" } else { "" },
                field(&l.label()),
            ));
        }
    }
    let mut out: BTreeMap<&'static str, String> = BTreeMap::new();
    out.insert("Knooppunt", knp);
    out.insert("Verbinding", vrb);
    out.insert("Profiel", format!("{PROFIEL}\n{}", profiles.into_values().collect::<String>()));
    for (name, header) in [("Kunstwerk", KUNSTWERK), ("Oppervlak", OPPERVLAK), ("Debiet", DEBIET), ("Verloop", VERLOOP), ("Nwrw", NWRW), ("ItObject", ITOBJECT)] {
        out.insert(name, format!("{header}\n"));
    }
    out.insert(
        "Meta",
        format!("{META}\n{};Open Infra Studio {};{date};;;{};Open Infra Studio (CIV_RIOOL_EXPORT FORMAAT=HYDX);Kunstwerken, oppervlakken en debieten zijn niet ingevuld\n", FILES.len(), env!("CARGO_PKG_VERSION"), field(description)),
    );
    Ok(out)
}

/// Writes the tables into `dir` (created); returns the files.
pub fn write(dir: &Path, tables: &BTreeMap<&'static str, String>) -> Result<Vec<PathBuf>, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out = Vec::new();
    for name in FILES {
        let path = dir.join(format!("{name}.csv"));
        std::fs::write(&path, tables.get(name).map(String::as_str).unwrap_or("")).map_err(|e| format!("{}: {e}", path.display()))?;
        out.push(path);
    }
    Ok(out)
}

/// Today as HydX writes dates (dd/mm/yyyy).
pub fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0) as i64;
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{d:02}/{m:02}/{y}")
}

#[cfg(test)]
mod tests {
    use super::super::riool::{Leiding, Put};
    use super::*;

    fn net() -> Net {
        let mut a = Put::new("P1".into(), [100.0, 200.0], Stelsel::Dwa, [1000.0, 0.0]);
        a.maaiveld = Some(1.25);
        let mut b = Put::new("P2".into(), [140.0, 200.0], Stelsel::Dwa, [800.0, 800.0]);
        b.maaiveld = Some(1.10);
        let mut l = Leiding::new(0, 1, 250.0, Materiaal::Pvc, Stelsel::Dwa);
        l.bob_van = -0.40;
        l.bob_naar = -0.52;
        Net { naam: "Riool 1".into(), putten: vec![a, b], leidingen: vec![l] }
    }

    #[test]
    fn hydx_tables() {
        let n = net();
        let t = tables(&[&n], &|p| Ok([p[0] + 100000.0, p[1] + 400000.0]), "04/11/2026", "test").unwrap();
        assert_eq!(t.len(), 10);
        let knp: Vec<&str> = t["Knooppunt"].lines().collect();
        assert_eq!(knp[0], KNOOPPUNT);
        let p1: Vec<&str> = knp[1].split(';').collect();
        assert_eq!(p1.len(), KNOOPPUNT.split(';').count());
        assert_eq!((p1[0], p1[3], p1[4], p1[6], p1[10], p1[11], p1[12], p1[14]), ("P1", "100100.00", "400200.00", "1.25", "RND", "-0.40", "1000", "INS"));
        let p2: Vec<&str> = knp[2].split(';').collect();
        assert_eq!((p2[10], p2[12], p2[13]), ("RHK", "800", "800"));
        let vrb: Vec<&str> = t["Verbinding"].lines().nth(1).unwrap().split(';').collect();
        assert_eq!(vrb.len(), VERBINDING.split(';').count());
        assert_eq!((vrb[1], vrb[2], vrb[3], vrb[5], vrb[6], vrb[7], vrb[8], vrb[9], vrb[11]), ("P1", "P2", "GSL", "-0.40", "-0.52", "OPN", "40.00", "DWA", "PVC250"));
        let pro: Vec<&str> = t["Profiel"].lines().nth(1).unwrap().split(';').collect();
        assert_eq!((pro[0], pro[1], pro[2]), ("PVC250", "PVC", "RND"));
        assert!(pro[3].parse::<f64>().unwrap() < 250.0, "inner diameter of a PVC DN 250");
        assert!(t["Meta"].lines().nth(1).unwrap().starts_with("10;Open Infra Studio"));
        assert_eq!(t["Kunstwerk"].lines().count(), 1);
    }

    #[test]
    fn duplicate_manhole_names_get_their_network() {
        let (a, mut b) = (net(), net());
        b.naam = "Riool 2".into();
        let t = tables(&[&a, &b], &|p| Ok(p), "04/11/2026", "").unwrap();
        assert!(t["Knooppunt"].contains("\nRiool 1_P1;") && t["Knooppunt"].contains("\nRiool 2_P1;"));
        assert!(t["Verbinding"].contains(";Riool 2_P1;Riool 2_P2;"));
    }

    #[test]
    fn date_format() {
        let d = today();
        assert_eq!(d.len(), 10);
        assert_eq!(&d[2..3], "/");
    }
}
