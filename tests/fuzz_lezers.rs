//! Fuzzing the file readers on stable Rust. The fixtures of the repository
//! are the seeds; each round mutates one (bytes, cuts, copies, repeats,
//! extreme numbers) and reads it on a thread with a plugin command's stack.
//! Inputs that a reader accepts join the pool, so later rounds go deeper.
//!
//! `cargo test --test fuzz_lezers` runs a short fixed round (a few seconds).
//! A long round, for one or more readers:
//!
//! ```text
//! FUZZ_SECONDS=600 FUZZ_ONLY=landxml,nlcspp FUZZ_OUT=/tmp/fuzz \
//!     cargo test --test fuzz_lezers -- --ignored --nocapture
//! ```
//!
//! The long round writes each input to `FUZZ_OUT/current-<reader>` before
//! reading it, so a stack overflow (which aborts) leaves its input behind.
//! A panic, a read longer than 10 s, or memory above 2 GB is saved as
//! `FUZZ_OUT/<panic|hang|memory>-<reader>-<n>`.

use std::panic;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use open_infra::civiel::as_profiel::Alignment;
use open_infra::civiel::landxml;
use open_infra::civiel::terrein::Tin;
use open_infra::gis::{bro, caps, citygml, cityjson, files, klic_extra, planning, qgis, rasterbestand, vector::Feature};

fn text(b: &[u8]) -> std::borrow::Cow<'_, str> {
    String::from_utf8_lossy(b)
}

fn nlcspp_xml(b: &[u8]) -> bool {
    let Ok(d) = nlcspp::xml::read(&text(b)) else { return false };
    let findings = nlcspp::check::check(&d, d.operator.as_deref());
    let _ = nlcspp::check::markdown(&findings, "fuzz");
    true
}

/// LandXML and the steps of `GIS_LANDXML` that need no drawing.
fn landxml(b: &[u8]) -> bool {
    let Ok(x) = landxml::parse(&text(b)) else { return false };
    for (_, faces) in &x.surfaces {
        let _ = Tin::from_faces(faces);
    }
    for a in &x.alignments {
        let al = Alignment { name: a.name.clone(), start: a.start, segs: a.segs.clone() };
        let _ = al.to_polyline_data(0.001);
    }
    true
}

fn file_data(r: Result<files::FileData, String>) -> bool {
    r.map(|d| {
        let _ = d.bounds();
        let _ = d.fields();
    })
    .is_ok()
}

fn geojson(b: &[u8]) -> bool {
    file_data(files::geojson(b))
}
fn gml(b: &[u8]) -> bool {
    file_data(files::gml(&text(b)))
}
fn kml(b: &[u8]) -> bool {
    file_data(files::kml(&text(b)))
}
fn gpx(b: &[u8]) -> bool {
    file_data(files::gpx(&text(b)))
}
fn csv(b: &[u8]) -> bool {
    file_data(files::csv(&text(b)))
}
fn wkb(b: &[u8]) -> bool {
    files::gpkg_geometry(b, &mut Feature::default())
}
fn cityjson(b: &[u8]) -> bool {
    cityjson::parse_with(&text(b), &|_| true, &|_| {}).is_ok()
}
fn citygml(b: &[u8]) -> bool {
    citygml::parse(&text(b)).is_ok()
}
fn imro(b: &[u8]) -> bool {
    let t = text(b);
    let _ = planning::gml::is_imro(&t);
    planning::gml::parse(&t).is_ok()
}
fn bro(b: &[u8]) -> bool {
    let t = text(b);
    let a = bro::parse_characteristics(&t).is_ok();
    let b = bro::parse_cpt(&t).is_ok();
    bro::parse_bhr(&t).is_ok() || a || b
}
fn klic(b: &[u8]) -> bool {
    klic_extra::parse(&text(b)).is_ok()
}
fn qgis(b: &[u8]) -> bool {
    qgis::parse(&text(b)).is_ok()
}
fn capabilities(b: &[u8]) -> bool {
    let t = text(b);
    let a = caps::parse_wms("https://example.org/wms", &t).is_ok();
    caps::parse_wfs("https://example.org/wfs", &t).is_ok() || a
}
fn asc(b: &[u8]) -> bool {
    rasterbestand::read_asc(&text(b)).is_ok()
}

struct Reader {
    name: &'static str,
    read: fn(&[u8]) -> bool,
    seeds: &'static [&'static [u8]],
}

macro_rules! fixture {
    ($p:literal) => {
        include_bytes!(concat!("fixtures/", $p)) as &[u8]
    };
}

const LANDXML_ALIGNMENT: &[u8] = br#"<?xml version="1.0"?>
<LandXML xmlns="http://www.landxml.org/schema/LandXML-1.2" version="1.2">
  <Units><Metric linearUnit="meter"/></Units>
  <CgPoints><CgPoint name="101" code="PUT">425000.0 105000.0 1.25</CgPoint><CgPoint name="s">0 0</CgPoint></CgPoints>
  <Surfaces><Surface name="Maaiveld"><Definition surfType="TIN">
    <Pnts><P id="1">0 0 1</P><P id="2">0 10 2</P><P id="3">10 10 3</P><P id="4">10 0 4</P></Pnts>
    <Faces><F>1 2 3</F><F>1 3 4</F><F i="1">2 3 4</F></Faces>
  </Definition></Surface></Surfaces>
  <Alignments><Alignment name="Hoofdas" staStart="100">
    <CoordGeom>
      <Line><Start pntRef="s"/><End>0 100</End></Line>
      <Curve rot="ccw" radius="50"><Start>0 100</Start><Center>50 100</Center><End>50 150</End></Curve>
      <Spiral length="30" radiusStart="INF" radiusEnd="200" rot="cw"><Start>50 150</Start><PI>60 150</PI><End>79.98313 150.74970</End></Spiral>
      <Spiral length="30" radiusStart="200" radiusEnd="INF" rot="ccw"><Start>79.98313 150.74970</Start><End>109.9 152.0</End></Spiral>
    </CoordGeom>
  </Alignment></Alignments>
</LandXML>"#;

const GEOJSON: &[u8] = br#"{"type":"FeatureCollection","crs":{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::28992"}},"features":[
{"type":"Feature","properties":{"n":1,"s":"a","x":null},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]],[[0.2,0.2],[0.4,0.2],[0.4,0.4],[0.2,0.2]]]}},
{"type":"Feature","properties":{},"geometry":{"type":"MultiLineString","coordinates":[[[0,0,1],[2,3,4]],[[5,5],[6,6]]]}},
{"type":"Feature","properties":{"k":[1,2]},"geometry":{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[2,3]},{"type":"MultiPoint","coordinates":[[1,1],[2,2]]},{"type":"MultiPolygon","coordinates":[[[[0,0],[1,0],[1,1],[0,0]]]]}]}}]}"#;

const READERS: &[Reader] = &[
    Reader { name: "nlcspp", read: nlcspp_xml, seeds: &[fixture!("nlcspp/alliander-demo.xml")] },
    Reader { name: "landxml", read: landxml, seeds: &[fixture!("landxml/site.xml"), fixture!("landxml/partial.xml"), LANDXML_ALIGNMENT] },
    Reader { name: "geojson", read: geojson, seeds: &[include_bytes!("data/gisbestanden/zurich.geojson"), GEOJSON] },
    Reader { name: "gml", read: gml, seeds: &[fixture!("gml/features.gml"), fixture!("gml/mixed.gml"), fixture!("gml/malformed.gml")] },
    Reader { name: "kml", read: kml, seeds: &[include_bytes!("data/gisbestanden/zurich.kml")] },
    Reader { name: "gpx", read: gpx, seeds: &[include_bytes!("data/gisbestanden/zurich.gpx")] },
    Reader { name: "csv", read: csv, seeds: &[fixture!("csv/points.csv"), fixture!("csv/multiline.csv"), include_bytes!("data/gisbestanden/zurich_lv95.csv")] },
    Reader { name: "wkb", read: wkb, seeds: &[fixture!("geopackage/point.wkb"), fixture!("geopackage/line.wkb"), fixture!("geopackage/circular.wkb")] },
    Reader { name: "cityjson", read: cityjson, seeds: &[fixture!("cityjson/building.city.json"), fixture!("cityjson/building.city.jsonl")] },
    Reader { name: "citygml", read: citygml, seeds: &[fixture!("citygml/building.gml"), fixture!("citygml/mixed.gml")] },
    Reader { name: "imro", read: imro, seeds: &[fixture!("planning/synthetic-imro.gml")] },
    Reader { name: "bro", read: bro, seeds: &[fixture!("bro/cpt-rejection.xml"), fixture!("bro/cpt-characteristics.xml"), fixture!("bro-synthetic/cpt.xml"), fixture!("bro-synthetic/borehole.xml")] },
    Reader { name: "klic", read: klic, seeds: &[fixture!("klic/GI_gebiedsinformatielevering_SYNTHETIC_1_V2.xml")] },
    Reader { name: "qgis", read: qgis, seeds: &[fixture!("qgis/wms.xml"), fixture!("qgis/wfs.xml"), fixture!("qgis/xyz.xml")] },
    Reader { name: "caps", read: capabilities, seeds: &[fixture!("catalogue-health/wms.xml"), fixture!("catalogue-health/wfs.xml")] },
    Reader { name: "asc", read: asc, seeds: &[fixture!("asc/terrain.asc"), fixture!("asc/nan-first-cell.asc")] },
];

/// xorshift64*: reproducible rounds without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
}

const NUMBERS: &[&[u8]] = &[
    b"NaN", b"nan", b"inf", b"-inf", b"INF", b"1e308", b"-1e308", b"1e999", b"1e-320", b"0", b"-0", b"-1", b"0.000000001",
    b"4294967296", b"18446744073709551616", b"9999999999999", b"1e15", b"-2147483649", b"3.4e38",
];
const TOKENS: &[&[u8]] = &[
    b"\"\"", b"null", b"[]", b"{}", b"[[[", b"]]]", b"<a>", b"</a>", b"/>", b"&amp;", b"&#0;", b"&#x10FFFF;", b"<![CDATA[x]]>",
    b"<!-- -->", b"\n", b"\r\n", b",", b";", b" ", b"\t", b"\"", b"'", b"\xef\xbb\xbf", b"\xff\xfe", b"\x00", b":", b"=",
];

fn mutate(rng: &mut Rng, base: &[u8], seeds: &[&[u8]]) -> Vec<u8> {
    let mut v = base.to_vec();
    for _ in 0..1 + rng.below(4) {
        let n = v.len();
        let at = rng.below(n + 1);
        let span = |rng: &mut Rng, max: usize| {
            let i = rng.below(n.max(1));
            (i, i + 1 + rng.below((n - i.min(n)).clamp(1, max)))
        };
        match rng.below(9) {
            0 if n > 0 => {
                let i = rng.below(n);
                v[i] = rng.next() as u8;
            }
            1 => {
                let t = TOKENS[rng.below(TOKENS.len())];
                v.splice(at..at, t.iter().copied());
            }
            2 => {
                let t = NUMBERS[rng.below(NUMBERS.len())];
                v.splice(at..at, t.iter().copied());
            }
            3 if n > 0 => {
                let (i, j) = span(rng, 64);
                v.drain(i..j.min(n)).for_each(drop);
            }
            4 if n > 0 => {
                // A copy elsewhere: repeated and nested structure.
                let (i, j) = span(rng, 512);
                let chunk = v[i..j.min(n)].to_vec();
                v.splice(at..at, chunk);
            }
            5 if n > 0 => v.truncate(rng.below(n)),
            6 if n > 0 => {
                // A number replaced by an extreme one.
                let from = rng.below(n);
                if let Some(p) = v[from..].iter().position(u8::is_ascii_digit) {
                    let a = from + p;
                    let e = a + v[a..].iter().position(|c| !(c.is_ascii_digit() || b".eE-+".contains(c))).unwrap_or(n - a);
                    let t = NUMBERS[rng.below(NUMBERS.len())];
                    v.splice(a..e, t.iter().copied());
                }
            }
            7 => {
                let s = seeds[rng.below(seeds.len())];
                if !s.is_empty() {
                    let i = rng.below(s.len());
                    let j = (i + 1 + rng.below(512)).min(s.len());
                    v.splice(at..at, s[i..j].iter().copied());
                }
            }
            8 if n > 0 => {
                // A short piece many times over.
                let (i, j) = span(rng, 16);
                let chunk = v[i..j.min(n)].repeat(1 + rng.below(300));
                v.splice(i..i, chunk);
            }
            _ => {}
        }
    }
    v.truncate(1 << 20);
    v
}

/// The last panic on a fuzz thread: message and place.
static PANIC: Mutex<Option<String>> = Mutex::new(None);
/// Milliseconds since `EPOCH` at which the current read started; 0 = idle.
static BUSY_SINCE: AtomicU64 = AtomicU64::new(0);

fn quiet_panics_on_fuzz_threads() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let default = panic::take_hook();
        panic::set_hook(Box::new(move |info| {
            if std::thread::current().name().is_some_and(|n| n.starts_with("fuzz-")) {
                *PANIC.lock().unwrap_or_else(|e| e.into_inner()) = Some(info.to_string());
            } else {
                default(info);
            }
        }));
    });
}

#[derive(Default)]
struct Round {
    reads: u64,
    accepted: u64,
    findings: Vec<String>,
}

/// Reads mutated inputs with `reader` until `rounds` or `until`, on a
/// thread with the 2 MiB stack of a plugin command.
fn fuzz(reader: &'static Reader, seed: u64, rounds: u64, until: Option<Instant>, out: Option<PathBuf>) -> Round {
    quiet_panics_on_fuzz_threads();
    let run = move || {
        let mut rng = Rng(seed ^ reader.name.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3)) | 1);
        let mut pool: Vec<Vec<u8>> = reader.seeds.iter().map(|s| s.to_vec()).collect();
        let mut round = Round::default();
        let epoch = Instant::now();
        while round.reads < rounds && until.is_none_or(|u| Instant::now() < u) {
            let base = &pool[rng.below(pool.len())];
            let input = mutate(&mut rng, base, reader.seeds);
            if let Some(out) = &out {
                let _ = std::fs::write(out.join(format!("current-{}", reader.name)), &input);
            }
            let start = Instant::now();
            BUSY_SINCE.store(1 + epoch.elapsed().as_millis() as u64, Ordering::SeqCst);
            let result = panic::catch_unwind(|| (reader.read)(&input));
            BUSY_SINCE.store(0, Ordering::SeqCst);
            round.reads += 1;
            let slow = start.elapsed() > Duration::from_secs(10);
            let finding = match result {
                Ok(_) if slow => Some(("hang", format!("{} read took {:?}", reader.name, start.elapsed()))),
                Ok(true) => {
                    round.accepted += 1;
                    if pool.len() < 256 { pool.push(input.clone()) } else { pool[rng.below(256)] = input.clone() }
                    None
                }
                Ok(false) => None,
                Err(_) => Some(("panic", PANIC.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default())),
            };
            if let Some((kind, what)) = finding {
                if !round.findings.iter().any(|f| f == &what) {
                    if let Some(out) = &out {
                        let _ = std::fs::write(out.join(format!("{kind}-{}-{}", reader.name, round.findings.len())), &input);
                    }
                    round.findings.push(what);
                }
            }
        }
        round
    };
    std::thread::Builder::new().name(format!("fuzz-{}", reader.name)).stack_size(2 << 20).spawn(run).unwrap().join().unwrap()
}

#[test]
fn readers_survive_mutated_fixtures() {
    let mut findings = Vec::new();
    for reader in READERS {
        let round = fuzz(reader, 0x5eed, 150, None, None);
        findings.extend(round.findings.into_iter().map(|f| format!("{}: {f}", reader.name)));
    }
    assert!(findings.is_empty(), "{findings:#?}");
}

/// Stops the process when memory goes above 2 GB, keeping the inputs
/// being read (`current-<reader>`) as a finding.
fn memory_watchdog(out: PathBuf, names: Vec<&'static str>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        let pages = std::fs::read_to_string("/proc/self/statm").ok().and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0);
        if pages * 4096 > 2 << 30 {
            for name in &names {
                let _ = std::fs::copy(out.join(format!("current-{name}")), out.join(format!("memory-{name}-watchdog")));
            }
            eprintln!("FUZZ memory: {} MB", pages * 4096 >> 20);
            std::process::exit(3);
        }
    });
}

#[test]
#[ignore = "long fuzz round; see the module documentation"]
fn fuzz_long() {
    let secs: u64 = std::env::var("FUZZ_SECONDS").ok().and_then(|s| s.parse().ok()).unwrap_or(600);
    let only = std::env::var("FUZZ_ONLY").unwrap_or_default();
    let seed = std::env::var("FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64);
    let out = PathBuf::from(std::env::var("FUZZ_OUT").unwrap_or_else(|_| "target/fuzz".into()));
    std::fs::create_dir_all(&out).unwrap();
    let chosen: Vec<&'static Reader> = READERS.iter().filter(|r| only.is_empty() || only.split(',').any(|o| o == r.name)).collect();
    memory_watchdog(out.clone(), chosen.iter().map(|r| r.name).collect());
    // One reader at a time, each with a guard for a read that does not end.
    for reader in chosen {
        let started = Instant::now();
        let until = started + Duration::from_secs(secs);
        let hang_guard = hang_guard(out.clone(), reader.name);
        let round = fuzz(reader, seed, u64::MAX, Some(until), Some(out.clone()));
        drop(hang_guard);
        eprintln!(
            "FUZZ {}: {} reads ({:.0}/s), {} accepted, seed {seed}, {} findings",
            reader.name,
            round.reads,
            round.reads as f64 / started.elapsed().as_secs_f64(),
            round.accepted,
            round.findings.len()
        );
        for f in &round.findings {
            eprintln!("FUZZ {} finding: {f}", reader.name);
        }
    }
}

/// While alive, stops the process when a single read of `name` passes 10 s.
fn hang_guard(out: PathBuf, name: &'static str) -> impl Drop {
    struct Guard(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = done.clone();
    std::thread::spawn(move || {
        let mut seen = (0u64, Instant::now());
        while !stop.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(250));
            let busy = BUSY_SINCE.load(Ordering::SeqCst);
            if busy == 0 || busy != seen.0 {
                seen = (busy, Instant::now());
            } else if seen.1.elapsed() > Duration::from_secs(10) {
                let _ = std::fs::copy(out.join(format!("current-{name}")), out.join(format!("hang-{name}-watchdog")));
                eprintln!("FUZZ hang: {name}: one read longer than 10 s");
                std::process::exit(4);
            }
        }
    });
    Guard(done)
}

