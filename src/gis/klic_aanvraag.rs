//! A KLIC request (gebiedsinformatie-aanvraag: oriëntatieverzoek,
//! graafmelding or calamiteitenmelding) from the drawing, as the GIA-POST
//! message of the Kadaster's B2B REST API (kadaster/klic-win, "Aanvragen
//! gebiedsinformatie", modelschema v0.89), checked locally with the
//! Kadaster's own rules before anything is sent.
//!
//! Sending is a paid, legal act in the requester's name: Open Infra only
//! prepares and checks the message (`GIS_KLIC_AANVRAAG`) and never sends it
//! by itself. The message follows the API's Swagger description
//! (service10.kadaster.nl/klic/api-docs, gia-post-api.json) and the
//! examples of the specification. The requester's details stay in the local config folder
//! (`klic-aanvrager.json`), never in the drawing.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::gis::crs::XY;
use crate::lang::tr;

pub const VALUES: &str = "http://definities.geostandaarden.nl/imkl2015/id/waarde";
const NLD: &str = "http://publications.europa.eu/resource/authority/country/NLD";

/// The kinds of work of the code list SoortWerkzaamhedenValue (BMKL 2.1).
pub const WERKZAAMHEDEN: &[&str] = &[
    "aardingSlaan", "archeologischOnderzoek", "baggerwerk", "bestratingswerk", "bodemonderzoekSonderingen", "bodemsanering", "bomenRooienOfPlanten",
    "bouwwerkzaamheden", "CAIkabelLeggen", "damwandBeschoeiingSlaan", "diepploegen", "drainage", "duikersLeggen", "funderingswerk", "gasleidingLeggen",
    "grondwerkBouwrijpMaken", "handholesPlaatsen", "HDPEbuisLeggen", "heien", "hekwerkPlaatsen", "hovenierswerkzaamheden", "huisaansluitingenMaken",
    "kabelsLeidingenLeggen", "kabelsLeidingenVerleggen", "leggenHoogspanning", "leggenLaagspanning", "leggenMiddenspanning", "mantelbuisLeggen",
    "OVwerkzaamheden", "palenMastenPlaatsenVerwijderen", "persingBoring", "proefsleuvenGraven", "reconstructiewerkzaamheden", "rioleringswerkzaamheden",
    "ruilverkaveling", "sloopwerkzaamheden", "stadsverwarmingLeggen", "stobbenFrezen", "tanksPuttenContainersInOfUitgraven", "telefoonkabelsLeggen",
    "trafostationPlaatsen", "verwijderenKabelsLeidingen", "voerkuilenGraven", "vijverGraven", "waterbouwkundigeWerken", "waterleidingLeggen",
    "wegenbouw", "werkAanBestaandeLeiding", "woningbouw", "woonrijpMaken", "zinkerMaken",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Soort {
    Orientatie,
    Graaf,
    Calamiteit,
}

impl Soort {
    pub fn parse(s: &str) -> Option<Soort> {
        match s.trim().to_ascii_lowercase().as_str() {
            "orientatie" | "oriëntatie" | "orientatieverzoek" | "oriëntatieverzoek" => Some(Soort::Orientatie),
            "graaf" | "graafmelding" => Some(Soort::Graaf),
            "calamiteit" | "calamiteitenmelding" => Some(Soort::Calamiteit),
            _ => None,
        }
    }
    fn uri(self) -> String {
        let v = match self {
            Soort::Orientatie => "orientatieverzoek",
            Soort::Graaf => "graafmelding",
            Soort::Calamiteit => "calamiteitenmelding",
        };
        format!("{VALUES}/AanvraagSoortValue/{v}")
    }
    /// Largest box of the drawn area (m): 2500 for an oriëntatieverzoek, else 500.
    pub fn max_size(self) -> f64 {
        if self == Soort::Orientatie { 2500.0 } else { 500.0 }
    }
}

/// An address as the API takes it (BAGid: the adresseerbaar object for the
/// work location, the nummeraanduiding for postal addresses).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Adres {
    #[serde(rename = "BAGid", default)]
    pub bag_id: String,
    #[serde(rename = "openbareRuimteNaam", default)]
    pub straat: String,
    #[serde(default)]
    pub huisnummer: String,
    #[serde(default)]
    pub huisletter: String,
    #[serde(default)]
    pub huisnummertoevoeging: String,
    #[serde(rename = "woonplaatsNaam", default)]
    pub woonplaats: String,
    #[serde(default)]
    pub postcode: String,
}

impl Adres {
    fn json(&self) -> Value {
        let mut m = serde_json::Map::new();
        for (k, v) in [("BAGid", &self.bag_id), ("openbareRuimteNaam", &self.straat), ("huisnummer", &self.huisnummer), ("huisletter", &self.huisletter), ("huisnummertoevoeging", &self.huisnummertoevoeging), ("woonplaatsNaam", &self.woonplaats), ("postcode", &self.postcode)] {
            if !v.trim().is_empty() {
                m.insert(k.into(), Value::String(v.trim().into()));
            }
        }
        m.insert("landcode".into(), Value::String(NLD.into()));
        Value::Object(m)
    }
}

/// A person and organisation (requester, client).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Partij {
    #[serde(default)]
    pub naam: String,
    #[serde(default)]
    pub telefoon: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub organisatie: String,
    #[serde(default)]
    pub kvk: String,
    #[serde(default)]
    pub adres: Adres,
    /// The requester's Kadaster relation number (10 digits).
    #[serde(default)]
    pub relatienummer: String,
}

/// The requester's details, kept locally (`<config>/open-infra/klic-aanvrager.json`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Aanvrager {
    #[serde(default)]
    pub aanvrager: Partij,
    /// The default client (opdrachtgever) of graafmeldingen.
    #[serde(default)]
    pub opdrachtgever: Partij,
    #[serde(default)]
    pub factuur_referentie: String,
}

pub fn aanvrager_file() -> std::path::PathBuf {
    super::servers::config_dir().join("klic-aanvrager.json")
}

/// The stored requester, or an empty template written on first use.
pub fn aanvrager() -> Aanvrager {
    let p = aanvrager_file();
    match std::fs::read_to_string(&p).ok().and_then(|s| serde_json::from_str(&s).ok()) {
        Some(a) => a,
        None => {
            let a = Aanvrager::default();
            let _ = std::fs::create_dir_all(super::servers::config_dir());
            if !p.exists() {
                let _ = std::fs::write(&p, serde_json::to_string_pretty(&a).unwrap_or_default());
            }
            a
        }
    }
}

/// One request.
#[derive(Clone, Debug, Default)]
pub struct Aanvraag {
    pub soort: Option<Soort>,
    pub referentie: String,
    pub start: String,
    pub eind: String,
    pub werkzaamheden: Vec<String>,
    pub omschrijving: String,
    pub locatie: Adres,
    pub locatie_omschrijving: String,
    /// The drawn area in RD (outer ring, not closed).
    pub polygoon: Vec<XY>,
    pub informatiepolygoon: Option<Vec<XY>>,
}

/// A finding of the local check, with the Kadaster's code where it has one.
#[derive(Clone, Debug, PartialEq)]
pub struct Melding {
    pub fout: bool,
    pub code: &'static str,
    pub tekst: String,
}

fn fout(code: &'static str, tekst: impl Into<String>) -> Melding {
    Melding { fout: true, code, tekst: tekst.into() }
}

fn segments_cross(a: XY, b: XY, c: XY, d: XY) -> bool {
    let o = |p: XY, q: XY, r: XY| ((q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])).signum();
    o(a, b, c) * o(a, b, d) < 0.0 && o(c, d, a) * o(c, d, b) < 0.0
}

/// The Kadaster's geometric checks of a polygon (1.5.2): RD, 2D, simple, no
/// repeated points, at most 3 decimals, at most `max_points`, within the
/// Netherlands, within a `max_size` box.
pub fn check_polygon(name: &str, ring: &[XY], max_points: usize, max_size: f64) -> Vec<Melding> {
    let mut out = Vec::new();
    let n = ring.len();
    if n < 3 {
        out.push(fout("COR1009", format!("{name}: {}", tr("minstens drie hoekpunten nodig", "at least three vertices needed"))));
        return out;
    }
    if n + 1 > max_points {
        out.push(fout("COR1053", format!("{name}: {} {} {max_points}", n + 1, tr("punten; het maximum is", "points; the maximum is"))));
    }
    for i in 0..n {
        for j in i + 1..n {
            if (ring[i][0] - ring[j][0]).abs() < 1e-3 && (ring[i][1] - ring[j][1]).abs() < 1e-3 {
                out.push(fout("COR1054", format!("{name}: {} {}", tr("herhaald punt", "repeated point"), j + 1)));
            }
        }
    }
    'outer: for i in 0..n {
        for j in i + 1..n {
            if j == i + 1 || (i == 0 && j == n - 1) {
                continue;
            }
            if segments_cross(ring[i], ring[(i + 1) % n], ring[j], ring[(j + 1) % n]) {
                out.push(fout("COR1055", format!("{name}: {}", tr("de polygoon kruist zichzelf", "the polygon crosses itself"))));
                break 'outer;
            }
        }
    }
    let (x0, x1) = ring.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[0]), b.max(p[0])));
    let (y0, y1) = ring.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p[1]), b.max(p[1])));
    if !(0.0..=300_000.0).contains(&x0) || !(0.0..=300_000.0).contains(&x1) || !(289_000.0..=629_000.0).contains(&y0) || !(289_000.0..=629_000.0).contains(&y1) {
        out.push(fout("COR1056", format!("{name}: {}", tr("ligt (deels) buiten Nederland (RD)", "lies (partly) outside the Netherlands (RD)"))));
    } else if ring.iter().any(|p| super::crs::transform("EPSG:28992", "EPSG:4326", *p).is_ok_and(|q| !super::crs::in_netherlands(q[0], q[1]))) {
        out.push(Melding { fout: false, code: "WAR", tekst: format!("{name}: {}", tr("een hoekpunt ligt mogelijk buiten de landsgrens", "a vertex may lie outside the national border")) });
    }
    if x1 - x0 > max_size || y1 - y0 > max_size {
        out.push(fout("COR1057", format!("{name}: {:.0} × {:.0} m; {} {max_size:.0} × {max_size:.0} m", x1 - x0, y1 - y0, tr("het maximum is", "the maximum is"))));
    }
    out
}

fn date_ok(s: &str) -> bool {
    s.len() == 10 && s.as_bytes()[4] == b'-' && s.as_bytes()[7] == b'-' && s[..4].parse::<u32>().is_ok() && s[5..7].parse::<u32>().is_ok_and(|m| (1..=12).contains(&m)) && s[8..].parse::<u32>().is_ok_and(|d| (1..=31).contains(&d))
}

fn email_ok(s: &str) -> bool {
    let Some((a, b)) = s.split_once('@') else { return false };
    !a.is_empty() && b.contains('.') && !b.starts_with('.') && !b.ends_with('.') && !s.contains(char::is_whitespace)
}

fn party_checks(who: &str, p: &Partij, address_code: &'static str, out: &mut Vec<Melding>) {
    if p.adres.bag_id.trim().is_empty() && (p.adres.straat.trim().is_empty() || p.adres.woonplaats.trim().is_empty()) {
        out.push(fout(address_code, format!("{who}: {}", tr("bezoekadres ontbreekt (BAG-id van de nummeraanduiding)", "visiting address missing (BAG id of the nummeraanduiding)"))));
    }
    let need = |v: &str, what: &str, out: &mut Vec<Melding>| {
        if v.trim().is_empty() {
            out.push(fout("COR1001", format!("{who}: {what} {}", tr("ontbreekt", "missing"))));
        }
    };
    need(&p.naam, tr("naam contactpersoon", "contact name"), out);
    need(&p.telefoon, tr("telefoon", "phone"), out);
    need(&p.email, "e-mail", out);
    need(&p.organisatie, tr("organisatie", "organisation"), out);
    if !p.email.is_empty() && !email_ok(&p.email) {
        out.push(fout("COR0111", format!("{who}: {} {}", tr("ongeldig e-mailadres", "invalid e-mail address"), p.email)));
    }
    for (v, max, what) in [(&p.naam, 100, "naam"), (&p.telefoon, 30, "telefoon"), (&p.email, 70, "email"), (&p.organisatie, 100, "organisatie"), (&p.kvk, 25, "kvkNummer")] {
        if v.chars().count() > max {
            out.push(fout("COR1002", format!("{who}: {what} {} {max} {}", tr("langer dan", "longer than"), tr("tekens", "characters"))));
        }
    }
}

/// The local check of a request against the Kadaster's rules (modelschema
/// cardinalities, lengths, code lists, dates, polygons). `today` is
/// yyyy-mm-dd.
pub fn check(a: &Aanvraag, who: &Aanvrager, today: &str) -> Vec<Melding> {
    let mut out = Vec::new();
    let Some(soort) = a.soort else {
        out.push(fout("COR0210", tr("soort aanvraag ontbreekt (oriëntatie, graaf of calamiteit)", "kind of request missing (orientatie, graaf or calamiteit)")));
        return out;
    };
    party_checks(tr("aanvrager", "requester"), &who.aanvrager, "COR0215", &mut out);
    let rel = &who.aanvrager.relatienummer;
    if rel.len() != 10 || !rel.chars().all(|c| c.is_ascii_digit()) {
        out.push(fout("COR0899", format!("{} ({})", tr("relatienummer van de aanvrager: 10 cijfers, met voorloopnullen", "requester's relation number: 10 digits, with leading zeros"), aanvrager_file().display())));
    }
    if soort == Soort::Graaf {
        party_checks(tr("opdrachtgever", "client"), &who.opdrachtgever, "COR0925", &mut out);
    }
    if a.referentie.trim().is_empty() {
        out.push(fout("COR0256", tr("referentie ontbreekt", "reference missing")));
    } else if a.referentie.chars().count() > 30 {
        out.push(fout("COR0256", tr("referentie langer dan 30 tekens", "reference longer than 30 characters")));
    }
    if soort != Soort::Calamiteit {
        if a.werkzaamheden.is_empty() || a.werkzaamheden.len() > 6 {
            out.push(fout("COR0190", tr("één tot zes soorten werkzaamheden nodig", "one to six kinds of work needed")));
        }
        for w in &a.werkzaamheden {
            if !WERKZAAMHEDEN.contains(&w.as_str()) {
                out.push(fout("COR0216", format!("{} {w}", tr("onbekende soort werkzaamheden:", "unknown kind of work:"))));
            }
        }
    }
    if a.omschrijving.chars().count() > 2000 {
        out.push(fout("COR0192", tr("omschrijving langer dan 2000 tekens", "description longer than 2000 characters")));
    }
    if soort == Soort::Graaf {
        if !date_ok(&a.start) || !date_ok(&a.eind) {
            out.push(fout("COR0182", tr("start- en einddatum (jjjj-mm-dd) zijn verplicht bij een graafmelding", "start and end date (yyyy-mm-dd) are required for a graafmelding")));
        } else {
            if a.start.as_str() <= today {
                out.push(fout("COR0182", tr("de startdatum moet in de toekomst liggen", "the start date must lie in the future")));
            }
            if a.eind < a.start {
                out.push(fout("COR0185", tr("de einddatum ligt voor de startdatum", "the end date lies before the start date")));
            }
        }
    }
    if a.locatie.bag_id.trim().is_empty() {
        out.push(fout("COR0818", tr("het adres van de locatie (BAG-id) ontbreekt", "the location's address (BAG id) is missing")));
    }
    let name = if soort == Soort::Orientatie { tr("oriëntatiepolygoon", "orientation polygon") } else { tr("graafpolygoon", "excavation polygon") };
    out.extend(check_polygon(name, &a.polygoon, 50, soort.max_size()));
    if let Some(info) = &a.informatiepolygoon {
        if soort == Soort::Orientatie {
            out.push(fout("COR1004", tr("een informatiepolygoon mag niet bij een oriëntatieverzoek", "an information polygon is not allowed with an orientatieverzoek")));
        }
        out.extend(check_polygon(tr("informatiepolygoon", "information polygon"), info, 200, 500.0));
    }
    out
}

fn ring_json(ring: &[XY]) -> Value {
    let r = |v: f64| (v * 1000.0).round() / 1000.0;
    let mut pts: Vec<Value> = ring.iter().map(|p| json!([r(p[0]), r(p[1])])).collect();
    pts.push(json!([r(ring[0][0]), r(ring[0][1])]));
    json!({"type": "Polygon", "crs": {"type": "name", "properties": {"name": "EPSG:28992"}}, "coordinates": [pts]})
}

fn party_json(p: &Partij, relatie: bool) -> Value {
    let mut org = json!({"naam": [p.organisatie.trim()], "bezoekAdres": p.adres.json()});
    if !p.kvk.trim().is_empty() {
        org["kvkNummer"] = json!(p.kvk.trim());
    }
    if relatie {
        org["relatienummer"] = json!(p.relatienummer.trim());
    }
    json!({"contactpersoon": {"naam": p.naam.trim(), "telefoon": p.telefoon.trim(), "email": p.email.trim()}, "organisatie": org})
}

/// The GIA-POST message (`postId`: a fresh UUID, so a retry is not processed twice).
pub fn message(a: &Aanvraag, who: &Aanvrager, post_id: &str) -> Value {
    let soort = a.soort.unwrap_or(Soort::Orientatie);
    let mut g = json!({
        "aanvrager": party_json(&who.aanvrager, true),
        "referentie": a.referentie.trim(),
        "aanvraagSoort": soort.uri(),
        "locatieWerkzaamheden": a.locatie.json(),
    });
    if soort == Soort::Graaf {
        g["opdrachtgever"] = party_json(&who.opdrachtgever, false);
        g["startDatum"] = json!(a.start);
        g["eindDatum"] = json!(a.eind);
    }
    if soort != Soort::Calamiteit {
        g["soortWerkzaamheden"] = json!(a.werkzaamheden.iter().map(|w| format!("{VALUES}/SoortWerkzaamhedenValue/{w}")).collect::<Vec<_>>());
    }
    if !a.omschrijving.trim().is_empty() {
        g["omschrijvingWerkzaamheden"] = json!(a.omschrijving.trim());
    }
    if !a.locatie_omschrijving.trim().is_empty() {
        g["locatieOmschrijving"] = json!(a.locatie_omschrijving.trim());
    }
    if soort == Soort::Orientatie {
        g["orientatiePolygoon"] = ring_json(&a.polygoon);
    } else {
        g["graafPolygoon"] = ring_json(&a.polygoon);
        if let Some(info) = &a.informatiepolygoon {
            g["informatiePolygoon"] = ring_json(info);
        }
    }
    let mut bestelling = json!({"postId": post_id});
    if !who.factuur_referentie.trim().is_empty() {
        bestelling["factuurReferentie"] = json!(who.factuur_referentie.trim());
    }
    json!({"bestelling": bestelling, "gebiedsinformatieAanvragen": [g]})
}

/// The nearest address of a point (RD) from the PDOK Locatieserver, with the
/// BAG id of its adresseerbaar object (what the API wants for the work location).
pub fn nearest_address(p: XY) -> Result<Adres, String> {
    let url = format!("https://api.pdok.nl/bzk/locatieserver/search/v3_1/reverse?X={:.2}&Y={:.2}&type=adres&rows=1&fl=*", p[0], p[1]);
    let text = super::http::get_text(&url)?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let d = v["response"]["docs"].get(0).ok_or_else(|| tr("geen adres gevonden bij het graafgebied", "no address found near the excavation area").to_string())?;
    let s = |k: &str| d[k].as_str().map(str::to_string).or_else(|| d[k].as_i64().map(|n| n.to_string())).unwrap_or_default();
    Ok(Adres { bag_id: s("adresseerbaarobject_id"), straat: s("straatnaam"), huisnummer: s("huisnummer"), huisletter: s("huisletter"), huisnummertoevoeging: s("huisnummertoevoeging"), woonplaats: s("woonplaatsnaam"), postcode: s("postcode") })
}

/// A UUID v4 from the system's randomness (for `postId`).
pub fn uuid() -> String {
    let mut b = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b)).is_ok();
    if !ok {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        b = t.to_le_bytes();
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: Vec<String> = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", h[0..4].concat(), h[4..6].concat(), h[6..8].concat(), h[8..10].concat(), h[10..16].concat())
}

/// The request from the command's arguments: `SOORT=orientatie|graaf|calamiteit
/// REFERENTIE= START=jjjj-mm-dd EIND= WERK=a,b OMSCHRIJVING= LOCATIE=`, with
/// the area (RD, outer ring) and the address nearest to its middle.
pub fn from_args(args: &crate::civiel::api::Args, ring: Vec<XY>, fallback_ref: &str) -> Aanvraag {
    let s = |k: &str| args.str(k).unwrap_or("").trim().to_string();
    let soort = args.str("SOORT").map_or(Some(Soort::Orientatie), Soort::parse);
    let werk: Vec<String> = s("WERK").split([',', ';']).map(str::trim).filter(|w| !w.is_empty()).map(|w| WERKZAAMHEDEN.iter().find(|k| k.eq_ignore_ascii_case(w)).map_or(w.to_string(), |k| k.to_string())).collect();
    let referentie = if s("REFERENTIE").is_empty() { fallback_ref.chars().take(30).collect() } else { s("REFERENTIE") };
    Aanvraag { soort, referentie, start: s("START"), eind: s("EIND"), werkzaamheden: werk, omschrijving: s("OMSCHRIJVING"), locatie: Adres::default(), locatie_omschrijving: s("LOCATIE"), polygoon: ring, informatiepolygoon: None }
}

fn centroid(ring: &[XY]) -> XY {
    let n = ring.len().max(1) as f64;
    [ring.iter().map(|p| p[0]).sum::<f64>() / n, ring.iter().map(|p| p[1]).sum::<f64>() / n]
}

/// Prepares and checks a request and writes it to `path` (the GIA-POST
/// message) and beside it the findings (`.md`). Nothing is sent.
pub fn prepare(mut a: Aanvraag, path: &std::path::Path) -> Result<String, String> {
    if a.polygoon.len() >= 3 && a.locatie.bag_id.is_empty() {
        match nearest_address(centroid(&a.polygoon)) {
            Ok(adr) => a.locatie = adr,
            Err(e) => crate::civiel::api::note("adres", e),
        }
    }
    let who = aanvrager();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let findings = check(&a, &who, &today);
    let errors = findings.iter().filter(|m| m.fout).count();
    let msg = message(&a, &who, &uuid());
    std::fs::write(path, serde_json::to_string_pretty(&msg).map_err(|e| e.to_string())?).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut md = format!("# {}: {}\n\n", tr("KLIC-aanvraag", "KLIC request"), a.referentie);
    md.push_str(&format!("{}\n\n", tr("Voorbereid en lokaal getoetst aan de regels van het Kadaster (GIA-API); niet ingediend.", "Prepared and checked locally against the Kadaster's rules (GIA API); not submitted.")));
    let l = &a.locatie;
    md.push_str(&format!("- {}: {} {}{}{}, {} {} (BAG {})\n", tr("locatie", "location"), l.straat, l.huisnummer, l.huisletter, if l.huisnummertoevoeging.is_empty() { String::new() } else { format!("-{}", l.huisnummertoevoeging) }, l.postcode, l.woonplaats, l.bag_id));
    md.push_str(&format!("- {}: {}\n\n", tr("aanvrager (lokaal)", "requester (local)"), aanvrager_file().display()));
    if findings.is_empty() {
        md.push_str(tr("Geen bevindingen.\n", "No findings.\n"));
    } else {
        md.push_str("| | Code | |\n|---|---|---|\n");
        for m in &findings {
            md.push_str(&format!("| {} | {} | {} |\n", if m.fout { tr("Fout", "Error") } else { tr("Let op", "Note") }, m.code, m.tekst.replace('|', "/")));
        }
    }
    let report = path.with_extension("md");
    let _ = std::fs::write(&report, md);
    crate::civiel::api::note("bestand", path.to_string_lossy().into_owned());
    crate::civiel::api::note("fouten", errors);
    let first: Vec<String> = findings.iter().filter(|m| m.fout).take(3).map(|m| format!("{} {}", m.code, m.tekst)).collect();
    Ok(if errors == 0 {
        format!("{}: {} ({})", tr("KLIC-aanvraag klaar om in te dienen, niet verzonden", "KLIC request ready to submit, not sent"), path.display(), report.display())
    } else {
        format!("{} {errors} {}: {}. {}: {}", tr("KLIC-aanvraag voorbereid met", "KLIC request prepared with"), tr("fout(en), nog niet indienbaar", "error(s), not yet submittable"), first.join("; "), tr("Bestand", "File"), report.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who() -> Aanvrager {
        let p = Partij { naam: "Ellen Graver".into(), telefoon: "088 123 45 67".into(), email: "ellen@voorbeeld.nl".into(), organisatie: "Voorbeeld B.V.".into(), kvk: "12345678".into(), adres: Adres { bag_id: "0505200000032217".into(), straat: "Lange Geldersekade".into(), huisnummer: "2".into(), woonplaats: "Dordrecht".into(), postcode: "3311CJ".into(), ..Default::default() }, relatienummer: "0000123456".into() };
        Aanvrager { aanvrager: p.clone(), opdrachtgever: p, factuur_referentie: String::new() }
    }

    fn square(x: f64, y: f64, w: f64) -> Vec<XY> {
        vec![[x, y], [x + w, y], [x + w, y + w], [x, y + w]]
    }

    fn graaf() -> Aanvraag {
        Aanvraag {
            soort: Some(Soort::Graaf),
            referentie: "Proefsleuf Grote Kerk".into(),
            start: "2027-01-10".into(),
            eind: "2027-01-12".into(),
            werkzaamheden: vec!["proefsleuvenGraven".into()],
            locatie: Adres { bag_id: "0505010000032217".into(), straat: "Lange Geldersekade".into(), huisnummer: "2".into(), woonplaats: "Dordrecht".into(), postcode: "3311CJ".into(), ..Default::default() },
            polygoon: square(104860.0, 425300.0, 40.0),
            ..Default::default()
        }
    }

    #[test]
    fn a_good_graafmelding_passes_and_its_message_follows_the_schema() {
        let f = check(&graaf(), &who(), "2026-11-04");
        assert!(f.iter().all(|m| !m.fout), "{f:#?}");
        let m = message(&graaf(), &who(), "id-1");
        let g = &m["gebiedsinformatieAanvragen"][0];
        assert_eq!(g["aanvraagSoort"], format!("{VALUES}/AanvraagSoortValue/graafmelding"));
        assert_eq!(g["graafPolygoon"]["crs"]["properties"]["name"], "EPSG:28992");
        let ring = g["graafPolygoon"]["coordinates"][0].as_array().unwrap();
        assert_eq!(ring.len(), 5, "closed");
        assert_eq!(ring[0], ring[4]);
        assert_eq!(g["aanvrager"]["organisatie"]["relatienummer"], "0000123456");
        assert!(g["opdrachtgever"]["organisatie"]["naam"].is_array());
        assert_eq!(g["soortWerkzaamheden"][0], format!("{VALUES}/SoortWerkzaamhedenValue/proefsleuvenGraven"));
        assert_eq!(m["bestelling"]["postId"], "id-1");
    }

    #[test]
    fn an_orientatieverzoek_has_no_client_dates_or_graafpolygoon() {
        let a = Aanvraag { soort: Some(Soort::Orientatie), start: String::new(), eind: String::new(), polygoon: square(104000.0, 425000.0, 2000.0), ..graaf() };
        assert!(check(&a, &Aanvrager { opdrachtgever: Partij::default(), ..who() }, "2026-11-04").iter().all(|m| !m.fout));
        let g = &message(&a, &who(), "x")["gebiedsinformatieAanvragen"][0];
        assert!(g.get("opdrachtgever").is_none() && g.get("startDatum").is_none() && g.get("graafPolygoon").is_none());
        assert!(g.get("orientatiePolygoon").is_some());
    }

    #[test]
    fn the_kadasters_rules() {
        let codes = |a: &Aanvraag| check(a, &who(), "2026-11-04").into_iter().filter(|m| m.fout).map(|m| m.code).collect::<Vec<_>>();
        assert!(codes(&Aanvraag { polygoon: square(104860.0, 425300.0, 600.0), ..graaf() }).contains(&"COR1057"), "a graafpolygoon over 500 m");
        assert!(codes(&Aanvraag { polygoon: vec![[104860.0, 425300.0], [104900.0, 425340.0], [104900.0, 425300.0], [104860.0, 425340.0]], ..graaf() }).contains(&"COR1055"), "a bow tie");
        let many: Vec<XY> = (0..60).map(|i| { let t = i as f64 / 60.0 * std::f64::consts::TAU; [104880.0 + 20.0 * t.cos(), 425320.0 + 20.0 * t.sin()] }).collect();
        assert!(codes(&Aanvraag { polygoon: many, ..graaf() }).contains(&"COR1053"), "more than 50 points");
        assert!(codes(&Aanvraag { polygoon: square(1_000_000.0, 425300.0, 40.0), ..graaf() }).contains(&"COR1056"), "outside the Netherlands");
        assert!(codes(&Aanvraag { start: "2026-11-01".into(), ..graaf() }).contains(&"COR0182"), "start in the past");
        assert!(codes(&Aanvraag { eind: "2027-01-01".into(), ..graaf() }).contains(&"COR0185"));
        assert!(codes(&Aanvraag { werkzaamheden: vec!["tuinieren".into()], ..graaf() }).contains(&"COR0216"));
        assert!(codes(&Aanvraag { referentie: "x".repeat(31), ..graaf() }).contains(&"COR0256"));
        let mut w = who();
        w.aanvrager.relatienummer = "123".into();
        assert!(check(&graaf(), &w, "2026-11-04").iter().any(|m| m.code == "COR0899"));
        w.aanvrager.adres = Adres::default();
        assert!(check(&graaf(), &w, "2026-11-04").iter().any(|m| m.code == "COR0215"));
        w.aanvrager.email = "geen-adres".into();
        assert!(check(&graaf(), &w, "2026-11-04").iter().any(|m| m.code == "COR0111"));
    }

    /// The property names of the API's Swagger description (gia-post-api.json).
    #[test]
    fn only_properties_of_the_swagger_description() {
        let adres = ["BAGid", "openbareRuimteNaam", "huisnummer", "huisletter", "huisnummertoevoeging", "woonplaatsNaam", "postcode", "landcode"];
        let polygoon = ["type", "crs", "coordinates"];
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        let within = |v: &Value, allowed: &[&str]| keys(v).iter().all(|k| allowed.contains(&k.as_str()));
        for a in [graaf(), Aanvraag { soort: Some(Soort::Orientatie), polygoon: square(104000.0, 425000.0, 2000.0), ..graaf() }] {
            let m = message(&Aanvraag { informatiepolygoon: Some(square(104850.0, 425290.0, 60.0)), ..a }, &who(), "x");
            assert!(within(&m, &["bestelling", "gebiedsinformatieAanvragen"]));
            assert!(within(&m["bestelling"], &["postId", "factuurReferentie", "factuurInkoopnummer"]));
            let g = &m["gebiedsinformatieAanvragen"][0];
            assert!(within(g, &["positienummer", "aanvrager", "referentie", "opdrachtgever", "aanvraagSoort", "voorbereidingCoordinatieCivieleWerken", "voorbereidingMedegebruikFysiekeInfrastructuur", "soortWerkzaamheden", "omschrijvingWerkzaamheden", "locatieWerkzaamheden", "locatieOmschrijving", "startDatum", "eindDatum", "huisaansluitingAdressen", "graafPolygoon", "orientatiePolygoon", "informatiePolygoon"]), "{:?}", keys(g));
            assert!(within(&g["aanvrager"], &["contactpersoon", "extraContact", "organisatie"]));
            assert!(within(&g["aanvrager"]["organisatie"], &["naam", "kvkNummer", "bezoekAdres", "relatienummer"]));
            assert!(within(&g["aanvrager"]["contactpersoon"], &["naam", "telefoon", "email"]));
            assert!(within(&g["aanvrager"]["organisatie"]["bezoekAdres"], &adres));
            assert!(within(&g["locatieWerkzaamheden"], &adres));
            if let Some(o) = g.get("opdrachtgever") {
                assert!(within(&o["organisatie"], &["naam", "kvkNummer", "bezoekAdres"]));
            }
            for k in ["graafPolygoon", "orientatiePolygoon", "informatiePolygoon"] {
                if let Some(p) = g.get(k) {
                    assert!(within(p, &polygoon));
                }
            }
        }
    }

    #[test]
    fn uuids() {
        let u = uuid();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[14..15], "4");
        assert_ne!(u, uuid());
    }
}
