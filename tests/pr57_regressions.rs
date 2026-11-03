//! Letterlijke reproducties van de drie fouten uit de review van PR #57.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ocs_plugin_api::host::{
    codec::{entities::Arc as CadArc, CadDocument, EntityType, Handle},
    PluginRequestError, PluginRequestSender,
};
use ocs_plugin_api::ipc::protocol::{PluginRequest, PluginResponse};
use open_infra::{
    basisdata::{self, KlicOptions},
    bridge,
    gis::{klic, klic_extra, site::Site},
    hoeveelheden,
};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("oi-pr57-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn zip(&self, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        for (name, bytes) in files {
            zip.start_file(*name, zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn pr57_arcs_with_endpoints_40_mm_apart_both_count() {
    for factor in [1.0, 1000.0] {
        let mut doc = CadDocument::new();
        for end in [1.0, 1.00004] {
            let mut arc = CadArc::from_coords(0.0, 0.0, 0.0, 1000.0 * factor, 0.0, end);
            arc.common.layer = "N-WE-VH-BANDEN-G".into();
            arc.common
                .extended_data
                .add_record(bridge::object_data([("id", "same-object".into())]));
            doc.add_entity(EntityType::Arc(arc)).unwrap();
        }
        let quantities = hoeveelheden::compute(&doc, factor, None);
        let row = &quantities.rows[0];
        assert!((row.length - 2000.04).abs() < 1e-8, "{row:?}");
        assert_eq!((row.count, row.duplicates, quantities.entities), (2, 0, 2));
        assert!(quantities.duplicates.is_empty());
    }
}

#[test]
fn pr57_same_pdf_name_keeps_each_operators_content() {
    let dir = TempDir::new("operators");
    let a = "bronnen/KL1001/voorwaarden.pdf";
    let b = "bronnen/KL1002/voorwaarden.pdf";
    let delivery = dir.zip(
        "levering.zip",
        &[(a, b"%PDF-1.4 KL1001"), (b, b"%PDF-1.4 KL1002")],
    );
    let cache = dir.0.join("cache");
    let first = klic_extra::attachment_file(&delivery, a, &cache).unwrap();
    let second = klic_extra::attachment_file(&delivery, b, &cache).unwrap();
    assert_eq!(std::fs::read(&first).unwrap(), b"%PDF-1.4 KL1001");
    assert_eq!(std::fs::read(&second).unwrap(), b"%PDF-1.4 KL1002");
    assert_ne!(first, second);
    assert!(first.ends_with(a) && second.ends_with(b));
    assert_eq!(
        klic_extra::attachment_file(&delivery, a, &cache).unwrap(),
        first
    );
}

#[test]
fn pr57_deliveries_with_the_same_zip_name_have_separate_caches() {
    let dir = TempDir::new("deliveries");
    let rel = "bronnen/KL1001/voorwaarden.pdf";
    let first_zip = dir.zip("first/levering.zip", &[(rel, b"%PDF-1.4 levering 1")]);
    let second_zip = dir.zip("second/levering.zip", &[(rel, b"%PDF-1.4 levering 2")]);
    let cache = dir.0.join("cache");
    let first = klic_extra::attachment_file(&first_zip, rel, &cache).unwrap();
    let second = klic_extra::attachment_file(&second_zip, rel, &cache).unwrap();
    assert_eq!(std::fs::read(&first).unwrap(), b"%PDF-1.4 levering 1");
    assert_eq!(std::fs::read(&second).unwrap(), b"%PDF-1.4 levering 2");
    assert_ne!(first, second);
}

fn detail_delivery(dir: &TempDir, geometry: &str) -> PathBuf {
    let xml = format!(
        r#"<gml:FeatureCollection xmlns:gml="http://www.opengis.net/gml/3.2" xmlns:imkl="http://www.geostandaarden.nl/imkl/2015/wion/2.0" xmlns:xlink="http://www.w3.org/1999/xlink">
      <gml:featureMember><imkl:ExtraDetailinfo gml:id="nl.imkl-KL1001.EXDI_1">
        <imkl:extraInfoType xlink:href="https://example.invalid/ExtraDetailInfoTypeValue/huisaansluiting"/>
        <imkl:bestandLocatie>bronnen/KL1001/detail.pdf</imkl:bestandLocatie>
        <imkl:ligging>{geometry}</imkl:ligging>
      </imkl:ExtraDetailinfo></gml:featureMember>
    </gml:FeatureCollection>"#
    );
    dir.zip(
        "levering.zip",
        &[
            ("GI_gebiedsinformatielevering_TEST_1.xml", xml.as_bytes()),
            ("bronnen/KL1001/detail.pdf", b"%PDF-1.4 detail"),
        ],
    )
}

fn assert_detail_attachment(delivery: &Path, extras: &klic_extra::Extras, cache: &Path) {
    assert_eq!(extras.attachments.len(), 1);
    let attachment = &extras.attachments[0];
    assert_eq!(attachment.path, "bronnen/KL1001/detail.pdf");
    assert_eq!(attachment.operator, "KL1001");
    let file = klic_extra::attachment_file(delivery, &attachment.path, cache).unwrap();
    assert_eq!(std::fs::read(file).unwrap(), b"%PDF-1.4 detail");
}

#[derive(Default)]
struct Drawing(Mutex<Vec<EntityType>>);

impl PluginRequestSender for Drawing {
    fn request(&self, request: PluginRequest) -> Result<PluginResponse, PluginRequestError> {
        Ok(match request {
            PluginRequest::AddLayer(_) => PluginResponse::OptHandle(Some(Handle::from(1000u64))),
            PluginRequest::TableOperation { .. } => {
                PluginResponse::TableResult(Ok(Handle::from(1001u64)))
            }
            PluginRequest::AddEntities(entities) => {
                let mut stored = self.0.lock().unwrap();
                let handles = (stored.len()..stored.len() + entities.len())
                    .map(|i| Handle::from(i as u64 + 1))
                    .collect();
                stored.extend(entities);
                PluginResponse::Handles(handles)
            }
            PluginRequest::ReadXRecord { .. } => PluginResponse::XRecordEntries(Ok(Vec::new())),
            PluginRequest::UpdateXRecord { .. } => {
                PluginResponse::XRecordUpdateResult(Ok(Handle::from(1002u64)))
            }
            PluginRequest::CreateXRecord { .. } => {
                PluginResponse::XRecordCreateResult(Ok(Handle::from(1002u64)))
            }
            PluginRequest::PushUndo { .. }
            | PluginRequest::WriteRecord { .. }
            | PluginRequest::SetDirty
            | PluginRequest::BumpGeometry => PluginResponse::Ok,
            other => {
                return Err(PluginRequestError(format!(
                    "onverwacht hostverzoek: {other:?}"
                )))
            }
        })
    }
}

#[test]
fn pr57_line_detail_keeps_its_geometry_and_openable_pdf() {
    let dir = TempDir::new("line-detail");
    let delivery = detail_delivery(&dir, "<gml:LineString srsName=\"EPSG:28992\"><gml:posList>100 200 110 210</gml:posList></gml:LineString>");
    let extras = klic_extra::read(&delivery).unwrap();
    assert_eq!(extras.count("extra_detailinfo"), 1);
    assert_detail_attachment(&delivery, &extras, &dir.0.join("cache"));
    let lev = klic::Levering {
        meldnummer: "TEST".into(),
        features: Vec::new(),
        themes: Default::default(),
        operators: Vec::new(),
        skipped: 0,
        extras,
        source: delivery,
    };
    let drawing = Arc::new(Drawing::default());
    let target = bridge::Target {
        sender: drawing.clone(),
        tab_id: 57001,
        unit_factor: 1.0,
        doc_path: None,
    };
    bridge::with_command_context(target, None, None, || {
        basisdata::klic_place(&Site::default(), &lev, &KlicOptions::default())
    })
    .unwrap();
    let stored = drawing.0.lock().unwrap();
    let lines: Vec<_> = stored
        .iter()
        .filter_map(|e| match e {
            EntityType::LwPolyline(p) => Some(p),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1, "{stored:?}");
    assert!(!lines[0].is_closed);
    assert!(lines[0].common.layer.contains("DETAIL1"));
    assert_eq!(
        lines[0]
            .vertices
            .iter()
            .map(|v| [v.location.x, v.location.y])
            .collect::<Vec<_>>(),
        vec![[100.0, 200.0], [110.0, 210.0]]
    );
}

#[test]
fn pr57_point_detail_lists_its_pdf_as_an_openable_attachment() {
    let dir = TempDir::new("point-detail");
    let delivery = detail_delivery(
        &dir,
        "<gml:Point srsDimension=\"3\"><gml:pos>105 405 -1.2</gml:pos></gml:Point>",
    );
    let extras = klic_extra::read(&delivery).unwrap();
    assert_eq!(extras.count("extra_detailinfo"), 1);
    assert_eq!(extras.areas[0].points, vec![[105.0, 405.0]]);
    assert_detail_attachment(&delivery, &extras, &dir.0.join("cache"));
}

fn assert_invalid_attachment_path(result: Result<PathBuf, String>, rel: &str) {
    assert_eq!(
        result.unwrap_err(),
        format!(
            "{} {rel}",
            open_infra::lang::tr("ongeldig bijlagepad", "invalid attachment path")
        )
    );
}

#[test]
fn pr58_absolute_attachment_path_cannot_write_outside_the_cache() {
    let dir = TempDir::new("absolute-write");
    let outside = dir.0.join("buiten-cache.pdf");
    let rel = outside.to_str().unwrap();
    let entry = format!("prefix{rel}");
    let delivery = dir.zip("levering.zip", &[(&entry, b"%PDF-1.4 bijlage")]);
    assert!(!outside.exists());
    let result = klic_extra::attachment_file(&delivery, rel, &dir.0.join("cache"));
    assert!(
        !outside.exists(),
        "de bijlage is buiten de cache geschreven"
    );
    assert_invalid_attachment_path(result, rel);
}

#[test]
fn pr58_absolute_attachment_path_cannot_return_an_existing_local_file() {
    let dir = TempDir::new("absolute-existing");
    let outside = dir.0.join("lokaal-bestand.pdf");
    std::fs::write(&outside, b"lokaal bestand, geen bijlage").unwrap();
    let rel = outside.to_str().unwrap();
    let entry = format!("prefix{rel}");
    let delivery = dir.zip("levering.zip", &[(&entry, b"%PDF-1.4 echte bijlage")]);
    let result = klic_extra::attachment_file(&delivery, rel, &dir.0.join("cache"));
    assert_eq!(
        std::fs::read(&outside).unwrap(),
        b"lokaal bestand, geen bijlage"
    );
    assert_invalid_attachment_path(result, rel);
}

#[test]
fn pr58_windows_attachment_prefixes_are_rejected_before_file_access() {
    let dir = TempDir::new("windows-paths");
    // Ook op Unix moeten leveringen met Windows-padnotaties worden geweigerd.
    // Een ontbrekende bron voorkomt toegang tot echte drives of UNC-shares.
    for rel in [
        r"C:\bronnen\voorwaarden.pdf",
        "C:/bronnen/voorwaarden.pdf",
        "C:voorwaarden.pdf",
        r"\\server\share\voorwaarden.pdf",
        "//server/share/voorwaarden.pdf",
        r"\bronnen\voorwaarden.pdf",
        r"\\?\C:\bronnen\voorwaarden.pdf",
        r"\\.\C:\bronnen\voorwaarden.pdf",
    ] {
        let result =
            klic_extra::attachment_file(&dir.0.join("ontbreekt.zip"), rel, &dir.0.join("cache"));
        assert_invalid_attachment_path(result, rel);
    }
}

#[cfg(unix)]
#[test]
fn pr58_cache_directory_symlink_cannot_write_outside_the_cache() {
    let dir = TempDir::new("directory-symlink");
    let seed = "bronnen/KL1001/seed.pdf";
    let rel = "bronnen/KL1002/voorwaarden.pdf";
    let delivery = dir.zip(
        "levering.zip",
        &[(seed, b"%PDF-1.4 seed"), (rel, b"%PDF-1.4 bijlage")],
    );
    let cache = dir.0.join("cache");
    let first = klic_extra::attachment_file(&delivery, seed, &cache).unwrap();
    let sources = first.parent().unwrap().parent().unwrap();
    let outside = dir.0.join("buiten-cache");
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, sources.join("KL1002")).unwrap();
    let result = klic_extra::attachment_file(&delivery, rel, &cache);
    assert!(
        !outside.join("voorwaarden.pdf").exists(),
        "de cache volgt een maplink naar buiten"
    );
    assert_invalid_attachment_path(result, rel);
}

#[cfg(unix)]
#[test]
fn pr58_cached_file_symlink_cannot_write_or_return_an_outside_file() {
    let dir = TempDir::new("file-symlink");
    let seed = "bronnen/KL1001/seed.pdf";
    let missing = "bronnen/KL1001/nieuw.pdf";
    let existing = "bronnen/KL1001/bestaand.pdf";
    let delivery = dir.zip(
        "levering.zip",
        &[
            (seed, b"%PDF-1.4 seed"),
            (missing, b"%PDF-1.4 nieuw"),
            (existing, b"%PDF-1.4 bestaand"),
        ],
    );
    let cache = dir.0.join("cache");
    let first = klic_extra::attachment_file(&delivery, seed, &cache).unwrap();
    for (rel, contents) in [
        (missing, None),
        (existing, Some(b"lokaal bestand".as_slice())),
    ] {
        let name = Path::new(rel).file_name().unwrap();
        let outside = dir.0.join(name);
        if let Some(bytes) = contents {
            std::fs::write(&outside, bytes).unwrap();
        }
        std::os::unix::fs::symlink(&outside, first.parent().unwrap().join(name)).unwrap();
        let result = klic_extra::attachment_file(&delivery, rel, &cache);
        assert_eq!(
            std::fs::read(&outside).ok().as_deref(),
            contents,
            "de cache schrijft via een bestandslink naar buiten"
        );
        assert_invalid_attachment_path(result, rel);
    }
}
