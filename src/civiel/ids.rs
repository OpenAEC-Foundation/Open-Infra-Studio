//! Checking an IFC model against a client's IDS (issue #65):
//! `CIV_IFC_IDS IDS=<bestand.ids> [IFC=<bestand.ifc>]`, and `CIV_IFC
//! IDS=<bestand.ids>` to export first and then check. The checking itself
//! is `rekenkern::ids` (IDS 1.0: entity, attribute, property,
//! classification, material and partOf facets, with cardinalities); this
//! module reads the files, writes the report beside the drawing
//! (`<tekening>_ids.md` and `<tekening>_ids.json`) and sums it up.

use std::path::{Path, PathBuf};

use rekenkern::ids::{Ids, IdsReport, Status};
use rekenkern::step::StepModel;

use crate::lang::tr;

/// A message in the interface language, with positional arguments.
macro_rules! t {
    ($nl:literal, $en:literal $(, $a:expr)* $(,)?) => {
        match crate::lang::current() {
            crate::lang::Lang::Nl => format!($nl $(, $a)*),
            crate::lang::Lang::En => format!($en $(, $a)*),
        }
    };
}

/// Checks the IFC text against the IDS text.
pub fn check_text(ids: &str, ifc: &[u8]) -> Result<IdsReport, String> {
    let ids = Ids::parse(ids).map_err(|e| format!("IDS: {e}"))?;
    let model = StepModel::parse(ifc).map_err(|e| format!("IFC: {e}"))?;
    Ok(ids.check(&model))
}

/// Checks the IFC file against the IDS file.
pub fn check_files(ids: &Path, ifc: &Path) -> Result<IdsReport, String> {
    let text = std::fs::read_to_string(ids).map_err(|e| format!("{}: {e}", ids.display()))?;
    let bytes = std::fs::read(ifc).map_err(|e| format!("{}: {e}", ifc.display()))?;
    check_text(&text, &bytes)
}

/// One line for the command line: the counts and the failing specifications.
pub fn summary(r: &IdsReport) -> String {
    let s = &r.summary;
    let mut line = t!(
        "IDS {}: {} van {} specificaties voldoen; {} van {} elementcontroles falen",
        "IDS {}: {} of {} specifications pass; {} of {} element checks fail",
        r.title,
        s.specifications_passed,
        s.specifications,
        s.checks_failed,
        s.checks
    );
    let failing: Vec<String> = r.specifications.iter().filter(|x| x.status == Status::Fail).map(|x| if x.failed > 0 { format!("{} ({})", x.name, x.failed) } else { x.name.clone() }).collect();
    if !failing.is_empty() {
        line.push_str(&t!(" — niet voldaan: {}", " — failing: {}", failing.join("; ")));
    }
    line
}

/// Writes the Markdown and JSON reports.
pub fn write_reports(r: &IdsReport, md: &Path, json: &Path) -> Result<(), String> {
    std::fs::write(md, r.to_markdown()).map_err(|e| format!("{}: {e}", md.display()))?;
    std::fs::write(json, r.to_json()).map_err(|e| format!("{}: {e}", json.display()))
}

/// Checks `ifc` against `ids`, writes the reports beside the drawing and
/// notes the counts for scripts and agents.
pub fn check_and_report(ids: &Path, ifc: &Path, progress: &dyn Fn(String)) -> Result<String, String> {
    progress(t!("IDS-controle van {}…", "IDS check of {}…", ifc.display()));
    let r = check_files(ids, ifc)?;
    let md = crate::bridge::beside_drawing("ids.md");
    let json = crate::bridge::beside_drawing("ids.json");
    write_reports(&r, &md, &json)?;
    super::api::note("ids", ids.to_string_lossy().into_owned());
    super::api::note("ifc", ifc.to_string_lossy().into_owned());
    super::api::note("rapport_md", md.to_string_lossy().into_owned());
    super::api::note("rapport_json", json.to_string_lossy().into_owned());
    super::api::note("voldoet", r.passed());
    super::api::note("specificaties", r.summary.specifications);
    super::api::note("specificaties_voldaan", r.summary.specifications_passed);
    super::api::note("specificaties_niet_voldaan", r.summary.specifications_failed);
    super::api::note("controles", r.summary.checks);
    super::api::note("controles_niet_voldaan", r.summary.checks_failed);
    super::api::note(
        "per_specificatie",
        r.specifications
            .iter()
            .map(|s| serde_json::json!({"naam": s.name, "voldoet": s.status == Status::Pass, "van_toepassing": s.applicable, "voldaan": s.passed, "niet_voldaan": s.failed, "opmerkingen": s.messages}))
            .collect::<Vec<_>>(),
    );
    Ok(format!("{} → {}", summary(&r), md.display()))
}

/// `CIV_IFC_IDS IDS=<bestand.ids> [IFC=<bestand.ifc>]`: without IFC= the
/// drawing's last export (`<tekening>_IFC4X3.ifc`), exported first when it
/// is not there.
pub fn command(ids: PathBuf, ifc: Option<PathBuf>, progress: &dyn Fn(String)) -> Result<String, String> {
    if !ids.is_file() {
        return Err(t!("IDS-bestand niet gevonden: {}", "IDS file not found: {}", ids.display()));
    }
    let (ifc, exported) = match ifc {
        Some(p) => (p, None),
        None => {
            let p = crate::bridge::beside_drawing("IFC4X3.ifc");
            if p.is_file() {
                (p, None)
            } else {
                progress(tr("Nog geen IFC-export: eerst exporteren…", "No IFC export yet: exporting first…").into());
                let m = crate::ifc::command(Some(p.clone()), false, true, progress)?;
                (p, Some(m))
            }
        }
    };
    let msg = check_and_report(&ids, &ifc, progress)?;
    Ok(match exported {
        Some(e) => format!("{e}\n{msg}"),
        None => msg,
    })
}

/// `CIV_IFC … IDS=<bestand.ids>`: export, then check the export.
pub fn export_and_check(path: Option<PathBuf>, selection: bool, texts: bool, ids: PathBuf, progress: &dyn Fn(String)) -> Result<String, String> {
    if !ids.is_file() {
        return Err(t!("IDS-bestand niet gevonden: {}", "IDS file not found: {}", ids.display()));
    }
    let path = path.unwrap_or_else(|| crate::bridge::beside_drawing("IFC4X3.ifc"));
    let exported = crate::ifc::command(Some(path.clone()), selection, texts, progress)?;
    let msg = check_and_report(&ids, &path, progress)?;
    Ok(format!("{exported}\n{msg}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ifc() -> Vec<u8> {
        let doc = crate::ifc::tests::sample();
        let site = crate::ifc::tests::sample_site();
        let src = crate::ifc::Source { doc: &doc, f: 1.0, site: Some(&site), name: "voorbeeld.dwg".into() };
        let mut report = crate::ifc::Report::default();
        crate::ifc::export(&src, &crate::ifc::Options::default(), Vec::new(), &mut report).unwrap()
    }

    /// Our own export contract (tests/data/ifc43-export.ids, which IfcTester
    /// checks in CI) passes; the report goes to target/ifc-test for the
    /// Python cross-check (tests/test_ifc_ids_crosscheck.py).
    #[test]
    fn the_export_meets_its_own_contract() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let ids = std::fs::read_to_string(root.join("tests/data/ifc43-export.ids")).unwrap();
        let r = check_text(&ids, &sample_ifc()).unwrap();
        assert!(r.passed(), "{}", r.to_markdown());
        assert!(r.summary.specifications == 2 && r.summary.checks > 10, "{:?}", r.summary);
        assert!(summary(&r).contains(&r.title));
        let dir = root.join("target/ifc-test");
        std::fs::create_dir_all(&dir).unwrap();
        write_reports(&r, &dir.join("voorbeeld_ids.md"), &dir.join("voorbeeld_ids.json")).unwrap();
        assert!(std::fs::read_to_string(dir.join("voorbeeld_ids.md")).unwrap().starts_with("# IDS check"));
    }

    #[test]
    fn a_requirement_the_export_misses_fails_with_its_reason() {
        let ids = r#"<?xml version="1.0" encoding="UTF-8"?>
<ids xmlns="http://standards.buildingsmart.org/IDS" xmlns:xs="http://www.w3.org/2001/XMLSchema">
  <info><title>Opdrachtgever</title></info>
  <specifications>
    <specification name="Gebouw met bouwjaar" ifcVersion="IFC4X3_ADD2">
      <applicability minOccurs="1" maxOccurs="unbounded"><entity><name><simpleValue>IFCBUILDING</simpleValue></name></entity></applicability>
      <requirements><property cardinality="required"><propertySet><simpleValue>Pset_BuildingCommon</simpleValue></propertySet><baseName><simpleValue>YearOfConstruction</simpleValue></baseName></property></requirements>
    </specification>
    <specification name="Assen" ifcVersion="IFC4X3_ADD2">
      <applicability minOccurs="1" maxOccurs="unbounded"><entity><name><simpleValue>IFCALIGNMENT</simpleValue></name></entity></applicability>
      <requirements><attribute cardinality="required"><name><simpleValue>Name</simpleValue></name></attribute></requirements>
    </specification>
  </specifications>
</ids>"#;
        let r = check_text(ids, &sample_ifc()).unwrap();
        assert!(!r.passed());
        assert_eq!((r.summary.specifications_passed, r.summary.specifications_failed), (1, 1));
        let building = &r.specifications[0];
        assert_eq!((building.applicable, building.failed), (1, 1));
        assert!(!building.elements[0].reasons.is_empty());
        assert_eq!(r.specifications[1].applicable, 2);
        assert!(summary(&r).contains("Gebouw met bouwjaar"));
        assert!(r.to_markdown().contains("Gebouw met bouwjaar"));
    }
}
