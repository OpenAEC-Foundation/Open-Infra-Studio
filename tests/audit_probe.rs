//! Audit probes (local only, not committed). Each test prints what it sees.
use std::io::Write;
use std::path::{Path, PathBuf};

use open_infra::gis::{caps, files, klic, klic_extra};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oi-audit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const GI_HEAD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;

fn gi_body(attachments: &[&str], extra: &str) -> String {
    let mut s = String::from(r#"<gml:FeatureCollection xmlns:gml="http://www.opengis.net/gml/3.2" xmlns:imkl="http://www.geostandaarden.nl/imkl/wibon" xmlns:xlink="http://www.w3.org/1999/xlink">
  <gml:featureMember>
    <imkl:Utiliteitsnet gml:id="nl.imkl-KL1031.UNET_LS">
      <imkl:thema xlink:href="http://definities.geostandaarden.nl/imkl2015/id/waarde/Thema/laagspanning"/>
    </imkl:Utiliteitsnet>
  </gml:featureMember>
  <gml:featureMember>
    <imkl:Elektriciteitskabel gml:id="nl.imkl-KL1031.KABEL_1">
      <imkl:inNetwork xlink:href="nl.imkl-KL1031.UNET_LS"/>
      <imkl:geometrie><gml:LineString gml:id="L1" srsName="urn:ogc:def:crs:EPSG::28992"><gml:posList>100 400 140 400</gml:posList></gml:LineString></imkl:geometrie>
    </imkl:Elektriciteitskabel>
  </gml:featureMember>
"#);
    for (i, a) in attachments.iter().enumerate() {
        s.push_str(&format!(r#"  <gml:featureMember>
    <imkl:Bijlage gml:id="nl.imkl-KL1081.Bijlage_{i}">
      <imkl:bijlageType xlink:href="http://definities.geostandaarden.nl/imkl2015/id/waarde/BijlageTypeValue/algemeen"/>
      <imkl:bestandLocatie>{a}</imkl:bestandLocatie>
    </imkl:Bijlage>
  </gml:featureMember>
"#));
    }
    s.push_str(extra);
    s.push_str("</gml:FeatureCollection>\n");
    s
}

fn zip_delivery(dir: &Path, xml: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let path = dir.join("Levering_99O000001_1.zip");
    let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("GI_gebiedsinformatielevering_99O000001_1_V2.xml", opt).unwrap();
    z.write_all(xml.as_bytes()).unwrap();
    for (name, bytes) in files {
        z.start_file(*name, opt).unwrap();
        z.write_all(bytes).unwrap();
    }
    z.finish().unwrap();
    path
}

/// KLIC attachments: any file type and any shell character is listed and extracted.
#[test]
fn probe_klic_attachment_types() {
    let dir = scratch("klic-att");
    let names = ["bronnen/KL1081/run.bat", "bronnen/KL1081/a&calc&.pdf", "bronnen/KL1081/setup.exe", "bronnen/KL1081/brief.pdf"];
    let xml = format!("{GI_HEAD}\n{}", gi_body(&names, ""));
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (*n, b"@echo off\r\necho probe\r\n" as &[u8])).collect();
    let zip = zip_delivery(&dir, &xml, &files);
    let lev = klic::read(&zip).unwrap();
    println!("meldnummer={} features={} attachments={:?}", lev.meldnummer, lev.features.len(), lev.extras.attachments.iter().map(|a| a.path.as_str()).collect::<Vec<_>>());
    let out = dir.join("out");
    for a in &lev.extras.attachments {
        let r = klic_extra::attachment_file(&lev.source, &a.path, &out);
        println!("attachment_file({:?}) -> {:?}", a.path, r);
    }
}

/// KLIC GI-xml with an internal DTD: entity expansion through the extras pass.
#[test]
fn probe_klic_dtd_expansion() {
    let dir = scratch("klic-dtd");
    let big = "x".repeat(100_000);
    let refs = "&a;".repeat(255);
    let doctype = format!("<!DOCTYPE gml:FeatureCollection [\n<!ENTITY a \"{big}\">\n<!ENTITY b \"{refs}\">\n]>");
    let body_refs = "&b;".repeat(20);
    let extra = format!("  <gml:featureMember><imkl:Bijlage gml:id=\"nl.imkl-KL1.B\"><imkl:omschrijving>{body_refs}</imkl:omschrijving></imkl:Bijlage></gml:featureMember>\n");
    let xml = format!("{GI_HEAD}\n{doctype}\n{}", gi_body(&[], &extra));
    println!("input GI-xml: {} bytes", xml.len());
    let zip = zip_delivery(&dir, &xml, &[]);
    println!("zip on disk: {} bytes", std::fs::metadata(&zip).unwrap().len());
    let t0 = std::time::Instant::now();
    let r = klic::read(&zip);
    println!("klic::read -> ok={} features={:?} in {:.1} s", r.is_ok(), r.as_ref().map(|l| l.features.len()), t0.elapsed().as_secs_f64());
    // The same text through the extras parser alone.
    let t0 = std::time::Instant::now();
    let x = klic_extra::parse(&xml);
    println!("klic_extra::parse -> ok={} in {:.1} s", x.is_ok(), t0.elapsed().as_secs_f64());
}

fn nested(open: &str, close: &str, depth: usize, inner: &str) -> String {
    let mut s = String::with_capacity((open.len() + close.len()) * depth + inner.len());
    for _ in 0..depth {
        s.push_str(open);
    }
    s.push_str(inner);
    for _ in 0..depth {
        s.push_str(close);
    }
    s
}

/// KML: MultiGeometry nested deeply.
#[test]
fn probe_kml_depth() {
    let depth: usize = std::env::var("PROBE_DEPTH").ok().and_then(|v| v.parse().ok()).unwrap_or(200_000);
    let geo = nested("<MultiGeometry>", "</MultiGeometry>", depth, "<Point><coordinates>4.9,52.3</coordinates></Point>");
    let text = format!("<?xml version=\"1.0\"?><kml xmlns=\"http://www.opengis.net/kml/2.2\"><Placemark>{geo}</Placemark></kml>");
    println!("KML: {} bytes, depth {depth}", text.len());
    let r = std::thread::Builder::new().stack_size(8 << 20).spawn(move || files::kml(&text).map(|d| d.features.len())).unwrap().join();
    println!("files::kml -> {:?}", r.map_err(|_| "panic"));
}

/// WMS capabilities (from a server): Layer nested deeply.
#[test]
fn probe_caps_depth() {
    let depth: usize = std::env::var("PROBE_DEPTH").ok().and_then(|v| v.parse().ok()).unwrap_or(200_000);
    let layers = nested("<Layer><Name>l</Name>", "</Layer>", depth, "");
    let xml = format!("<?xml version=\"1.0\"?><WMS_Capabilities version=\"1.3.0\" xmlns=\"http://www.opengis.net/wms\"><Service><Title>t</Title></Service><Capability><Request><GetMap><Format>image/png</Format><DCPType><HTTP><Get><OnlineResource xmlns:xlink=\"http://www.w3.org/1999/xlink\" xlink:href=\"https://example.invalid/wms?\"/></Get></HTTP></DCPType></GetMap></Request>{layers}</Capability></WMS_Capabilities>");
    println!("capabilities: {} bytes, depth {depth}", xml.len());
    let r = std::thread::Builder::new().stack_size(8 << 20).spawn(move || caps::parse_wms("https://example.invalid/wms", &xml).map(|c| c.layers.len())).unwrap().join();
    println!("caps::parse_wms -> {:?}", r.map_err(|_| "panic"));
}

/// KMZ: a small zip with a large KML (zip bomb).
#[test]
fn probe_kmz_bomb() {
    let dir = scratch("kmz");
    let path = dir.join("bom.kmz");
    let size: usize = std::env::var("PROBE_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(512) << 20;
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
        z.start_file("doc.kml", opt).unwrap();
        z.write_all(b"<kml>").unwrap();
        let chunk = vec![b' '; 1 << 20];
        for _ in 0..(size >> 20) {
            z.write_all(&chunk).unwrap();
        }
        z.write_all(b"</kml>").unwrap();
        z.finish().unwrap();
    }
    println!("kmz on disk: {} bytes, kml inside: {} bytes", std::fs::metadata(&path).unwrap().len(), size + 11);
    let t0 = std::time::Instant::now();
    let r = files::kmz(&path).map(|d| d.features.len());
    println!("files::kmz -> {:?} in {:.1} s", r, t0.elapsed().as_secs_f64());
}

