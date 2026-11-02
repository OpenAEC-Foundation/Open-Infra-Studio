//! Service browser: every map service Open Infra knows — the built-in presets,
//! the worldwide list from GIS2BIM (`assets/servers/world.json`) and the
//! user's own servers — searchable by free text in Dutch and English, with
//! filters on category, service type and coverage.
//!
//! Coverage is a bounding box in WGS 84 per service: the state, province, city
//! or country its data covers (see `tools/import_gis2bim_servers.py`). "At
//! this location" keeps the services whose box contains a point.

use serde_json::{json, Value};

use super::crs::{self, Support};
use super::servers::{self, MapServer, ServerKind};

pub type LonLat = [f64; 2];

/// Where an entry comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Builtin,
    World,
    User,
}

#[derive(Clone, Debug)]
pub struct Service {
    pub id: String,
    pub kind: ServerKind,
    pub name: String,
    pub provider: String,
    /// ISO country code, `EU`, `AFRICA`, `GLOBAL`; empty for user servers.
    pub country: String,
    pub region_nl: String,
    pub region_en: String,
    pub coverage: String,
    pub category: String,
    pub url: String,
    pub default_layer: String,
    pub crs: Vec<String>,
    pub license: String,
    pub terms: String,
    pub attribution: String,
    /// WGS 84 [west, south, east, north]; `None` = unknown (user servers).
    pub bbox: Option<[f64; 4]>,
    pub source: Source,
    /// Folded text for the search (name, provider, coverage, layer, host).
    hay: String,
}

/// Categories with their Dutch and English labels and search words (folded:
/// lower case, no accents). A query word that is one of these words asks
/// for the category.
pub const CATEGORIES: &[(&str, &str, &str, &[&str])] = &[
    ("cadastre", "Kadaster / percelen", "Cadastre / parcels", &[
        "kadaster", "kadastraal", "kadastrale", "perceel", "percelen", "perceelgrens", "brk", "cadastre", "cadastral", "cadaster",
        "parcel", "parcels", "land registry", "property", "flurstuck", "flurstucke", "flurstuecke", "kataster", "liegenschaft",
        "liegenschaftskataster", "alkis", "grundstuck", "cadastro", "catastro", "catasto", "parcela", "parcelas", "parcelle",
        "parcelles", "particella", "matrikel", "fastighet", "kiinteisto", "katastr", "dzialka", "dzialki", "lot", "lots",
    ]),
    ("aerial", "Luchtfoto / satelliet", "Aerial / satellite", &[
        "luchtfoto", "luchtfotos", "orthofoto", "ortho", "satelliet", "aerial", "orthophoto", "orthophotos", "orthoimagery", "imagery",
        "satellite", "luftbild", "luftbilder", "dop", "ortofoto", "ortofotos", "orthophotographie", "photo",
    ]),
    ("elevation", "Hoogte / terrein", "Elevation / terrain", &[
        "hoogte", "hoogtes", "hoogtekaart", "hoogtelijnen", "terrein", "maaiveld", "ahn", "elevation", "height", "heights", "dem", "dtm", "dsm",
        "terrain", "relief", "hillshade", "lidar", "contour", "contours", "gelande", "hohe", "hohen", "dgm", "dom", "mdt", "mde",
        "altimetria", "bathymetry", "bathymetrie",
    ]),
    ("subsurface", "Bodem / geologie", "Subsurface / geology", &[
        "bodem", "ondergrond", "geologie", "geologisch", "grondwater", "boringen", "sonderingen", "bro", "dino", "subsurface", "geology",
        "geological", "soil", "soils", "borehole", "boreholes", "groundwater", "geotechnical", "boden", "geologia", "geologie",
    ]),
    ("zoning", "Ruimtelijke plannen", "Zoning / planning", &[
        "bestemmingsplan", "bestemmingsplannen", "omgevingsplan", "ruimtelijke", "plannen", "zoning", "planning", "land use plan",
        "bebauungsplan", "bauleitplan", "bauleitplanung", "flachennutzungsplan", "urbanismo", "planeamiento", "plu", "lokalplan",
    ]),
    ("buildings", "Gebouwen", "Buildings", &[
        "gebouw", "gebouwen", "pand", "panden", "bag", "building", "buildings", "footprint", "footprints", "gebaude", "hausumringe",
        "edificio", "edificios", "batiment", "batiments", "bygning", "byggnad", "adressen", "addresses",
    ]),
    ("buildings3d", "3D-gebouwen", "3D buildings", &["3d", "3d-gebouwen", "lod2", "lod1", "citygml", "3d buildings", "3d-gebaude", "plateau"]),
    ("basemap", "Achtergrondkaart", "Basemap", &[
        "achtergrondkaart", "basiskaart", "topografie", "topografisch", "basemap", "base map", "topographic", "topography", "topo",
        "webatlas", "topographie", "openstreetmap", "osm",
    ]),
    ("environment", "Milieu / natuur", "Environment", &[
        "milieu", "natuur", "geluid", "luchtkwaliteit", "environment", "environmental", "noise", "air quality", "ecology", "umwelt", "habitat",
    ]),
    ("roads", "Wegen", "Roads", &["weg", "wegen", "straat", "straten", "nwb", "verkeer", "road", "roads", "street", "streets", "traffic", "strasse", "strassen", "carreteras", "routes"]),
    ("protected", "Beschermde gebieden", "Protected areas", &[
        "beschermd", "beschermde", "natura2000", "natura", "monument", "monumenten", "erfgoed", "protected", "heritage", "reserve",
        "schutzgebiet", "schutzgebiete", "denkmal",
    ]),
    ("water", "Water", "Water", &[
        "water", "waterlopen", "waterlopen", "rivier", "rivieren", "vaarwegen", "hydrografie", "overstroming", "hydrography", "river", "rivers",
        "flood", "flooding", "waterways", "gewasser", "hydrografia",
    ]),
    ("rail", "Spoor", "Rail", &["spoor", "spoorweg", "spoorwegen", "trein", "rail", "railway", "railways", "railroad", "bahn", "eisenbahn", "ferrocarril"]),
    ("landcover", "Bodemgebruik", "Land cover", &["bodemgebruik", "landgebruik", "bodembedekking", "vegetatie", "landcover", "land cover", "landuse", "land use", "corine", "vegetation", "landnutzung"]),
    ("admin", "Bestuurlijke grenzen", "Administrative", &[
        "bestuurlijk", "bestuurlijke", "grenzen", "gemeente", "gemeentegrenzen", "provincie", "administrative", "boundaries", "boundary",
        "municipality", "municipalities", "admin", "verwaltungsgrenzen", "gemeinde", "limites",
    ]),
    ("structures", "Kunstwerken", "Structures", &["kunstwerk", "kunstwerken", "brug", "bruggen", "viaduct", "tunnel", "structures", "bridge", "bridges", "tunnels", "dams", "brucken"]),
    ("utilities", "Kabels en leidingen", "Utilities", &["kabels", "leidingen", "nuts", "riolering", "riool", "energie", "utilities", "pipelines", "pipeline", "cables", "sewer", "power", "electricity", "leitungen"]),
];

/// Search words per country or region code: Dutch, English, native names.
const COUNTRY_WORDS: &[(&str, &str)] = &[
    ("AR", "argentinie argentina"), ("AT", "oostenrijk austria osterreich autriche"), ("AU", "australie australia"),
    ("BE", "belgie belgium belgique belgien"), ("BG", "bulgarije bulgaria"), ("BR", "brazilie brazil brasil"),
    ("CA", "canada"), ("CH", "zwitserland switzerland schweiz suisse svizzera"), ("CL", "chili chile"), ("CO", "colombia"),
    ("CZ", "tsjechie czechia czech cesko"), ("DE", "duitsland germany deutschland allemagne alemania"), ("DK", "denemarken denmark danmark"),
    ("EE", "estland estonia eesti"), ("ES", "spanje spain espana espagne"), ("FI", "finland suomi"), ("FR", "frankrijk france"),
    ("GB", "verenigd koninkrijk united kingdom uk britain brittannie engeland england schotland scotland wales"),
    ("GR", "griekenland greece hellas"), ("HK", "hongkong hong kong"), ("HR", "kroatie croatia hrvatska"), ("ID", "indonesie indonesia"),
    ("IE", "ierland ireland eire"), ("IL", "israel"), ("IN", "india"), ("IS", "ijsland iceland island"), ("IT", "italie italy italia"),
    ("JP", "japan nippon"), ("LI", "liechtenstein"), ("LT", "litouwen lithuania lietuva"), ("LU", "luxemburg luxembourg"),
    ("LV", "letland latvia latvija"), ("MX", "mexico"), ("NL", "nederland netherlands holland"), ("NO", "noorwegen norway norge"),
    ("NZ", "nieuw-zeeland new zealand aotearoa"), ("PE", "peru"), ("PH", "filipijnen philippines"), ("PL", "polen poland polska"),
    ("PT", "portugal"), ("SE", "zweden sweden sverige"), ("SG", "singapore"), ("SI", "slovenie slovenia slovenija"),
    ("SK", "slowakije slovakia slovensko"), ("TH", "thailand"), ("TW", "taiwan"), ("US", "verenigde staten united states usa america amerika"),
    ("UY", "uruguay"), ("ZA", "zuid-afrika south africa"), ("AFRICA", "afrika africa"), ("EU", "europa europe eu"),
    ("GLOBAL", "wereld world worldwide wereldwijd global globaal"),
];

/// Place names in other languages → the English name in the coverage texts.
const PLACE_WORDS: &[(&str, &str)] = &[
    ("beieren", "bavaria"), ("bayern", "bavaria"), ("noordrijn-westfalen", "north rhine-westphalia"), ("nordrhein-westfalen", "north rhine-westphalia"),
    ("nrw", "north rhine-westphalia"), ("hessen", "hesse"), ("sachsen", "saxony"), ("saksen", "saxony"), ("nedersaksen", "niedersachsen"),
    ("thuringen", "thuringia"), ("rijnland-palts", "rhineland-palatinate"), ("rheinland-pfalz", "rhineland-palatinate"),
    ("sleeswijk-holstein", "schleswig-holstein"), ("munchen", "munich"), ("keulen", "cologne"), ("koln", "cologne"),
    ("vlaanderen", "flanders"), ("wallonie", "wallonia"), ("brussel", "brussels"), ("bruxelles", "brussels"),
    ("wenen", "vienna"), ("wien", "vienna"), ("tirol", "tyrol"), ("steiermark", "styria"), ("karinthie", "carinthia"), ("karnten", "carinthia"),
    ("neder-oostenrijk", "lower austria"), ("niederosterreich", "lower austria"), ("zurich", "zurich"),
    ("praag", "prague"), ("praha", "prague"), ("kopenhagen", "copenhagen"), ("kobenhavn", "copenhagen"), ("engeland", "england"),
    ("schotland", "scotland"), ("noord-ierland", "northern ireland"), ("lissabon", "lisbon"), ("lisboa", "lisbon"),
    ("catalonie", "catalonia"), ("catalunya", "catalonia"), ("andalusie", "andalusia"), ("baskenland", "basque"),
    ("zuid-tirol", "south tyrol"), ("sudtirol", "south tyrol"), ("toscane", "tuscany"), ("toscana", "tuscany"),
    ("kaapstad", "cape town"), ("nieuw-zuid-wales", "new south wales"), ("west-australie", "western australia"),
    ("tasmanie", "tasmania"), ("brits-columbia", "british columbia"), ("new york", "new york"), ("sao", "sao paulo"),
];

/// Words that say nothing about what to find, dropped when others remain.
const STOP_WORDS: &[&str] = &[
    "in", "de", "het", "een", "en", "van", "voor", "met", "op", "bij", "the", "of", "and", "for", "with", "at", "near", "all", "alle",
    "kaart", "kaarten", "map", "maps", "karte", "service", "services", "dienst", "diensten", "data", "layer", "layers", "laag", "lagen",
];

/// Query words that ask for a service type.
fn kind_for(word: &str) -> Option<ServerKind> {
    match word {
        "wms" | "wmts" => Some(ServerKind::Wms),
        "wfs" | "vector" | "vectordata" => Some(ServerKind::Wfs),
        "xyz" | "tms" | "tiles" | "tegels" => Some(ServerKind::Xyz),
        _ => None,
    }
}

/// Lower case without accents: "Flurstücke" → "flurstucke".
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' => out.push('a'),
            'æ' => out.push_str("ae"),
            'ç' | 'č' | 'ć' => out.push('c'),
            'ď' => out.push('d'),
            'è' | 'é' | 'ê' | 'ë' | 'ě' | 'ę' | 'ē' => out.push('e'),
            'ì' | 'í' | 'î' | 'ï' | 'ī' => out.push('i'),
            'ł' | 'ľ' | 'ĺ' => out.push('l'),
            'ñ' | 'ň' | 'ń' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ő' | 'ō' => out.push('o'),
            'œ' => out.push_str("oe"),
            'ř' | 'ŕ' => out.push('r'),
            'š' | 'ś' | 'ș' | 'ş' => out.push('s'),
            'ß' => out.push_str("ss"),
            'ť' | 'ț' | 'ţ' => out.push('t'),
            'ù' | 'ú' | 'û' | 'ü' | 'ů' | 'ű' | 'ū' => out.push('u'),
            'ý' | 'ÿ' => out.push('y'),
            'ž' | 'ź' | 'ż' => out.push('z'),
            c => out.push(c),
        }
    }
    out
}

/// Query words: folded, split on spaces and punctuation (hyphens stay), stop
/// words dropped when other words remain.
pub fn words(text: &str) -> Vec<String> {
    let all: Vec<String> = fold(text)
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == ':'))
        .map(|w| w.trim_matches('-').to_string())
        .filter(|w| !w.is_empty())
        .collect();
    let kept: Vec<String> = all.iter().filter(|w| !STOP_WORDS.contains(&w.as_str()) && w.chars().count() > 1).cloned().collect();
    if kept.is_empty() { all } else { kept }
}

/// Dutch or English label of a category id.
pub fn category_label(id: &str) -> String {
    CATEGORIES
        .iter()
        .find(|c| c.0 == id)
        .map(|c| crate::lang::tr(c.1, c.2).to_string())
        .unwrap_or_else(|| if id.is_empty() { "—".into() } else { id.to_string() })
}

fn synonym_match(word: &str, syn: &str) -> bool {
    word == syn || (word.len() >= 4 && syn.starts_with(word)) || (syn.len() >= 5 && word.starts_with(syn))
}

/// The categories a query word names ("percelen" → cadastre).
pub fn categories_for(word: &str) -> Vec<&'static str> {
    CATEGORIES.iter().filter(|c| c.0 == word || c.3.iter().any(|s| synonym_match(word, s))).map(|c| c.0).collect()
}

/// The countries or regions a query word names ("duitsland" → DE).
fn countries_for(word: &str) -> Vec<&'static str> {
    COUNTRY_WORDS
        .iter()
        .filter(|(code, names)| {
            word.eq_ignore_ascii_case(code) || (word.len() >= 3 && names.split(' ').any(|n| n == word || (word.len() >= 4 && n.starts_with(word))))
        })
        .map(|(c, _)| *c)
        .collect()
}

/// Coverage boxes for the built-in presets (not in world.json).
const NL_BBOX: [f64; 4] = [3.2, 50.75, 7.23, 53.6];
const FLANDERS_BBOX: [f64; 4] = [2.54, 50.68, 5.92, 51.51];
const WORLD_BBOX: [f64; 4] = [-180.0, -90.0, 180.0, 90.0];

/// Built-in presets: id → (category, licence, coordinate systems).
fn builtin_meta(id: &str) -> (&'static str, &'static str, &'static [&'static str]) {
    const PDOK: &[&str] = &["EPSG:28992", "EPSG:3857", "EPSG:4258", "EPSG:4326", "EPSG:25831", "EPSG:25832"];
    const VL: &[&str] = &["EPSG:31370", "EPSG:3812", "EPSG:3857", "EPSG:4326"];
    match id {
        "pdok-luchtfoto" => ("aerial", "CC BY 4.0", PDOK),
        "pdok-kadastralekaart-wms" | "pdok-kadastralekaart-wfs" => ("cadastre", "CC BY 4.0", PDOK),
        "pdok-bag-wms" | "pdok-bag-wfs" => ("buildings", "CC0 1.0", PDOK),
        "pdok-top10nl-wms" => ("basemap", "CC BY 4.0", PDOK),
        "pdok-ahn-wms" => ("elevation", "CC0 1.0", PDOK),
        "pdok-nwbwegen-wfs" => ("roads", "CC0 1.0", PDOK),
        "pdok-nwbvaarwegen-wfs" => ("water", "CC0 1.0", PDOK),
        "pdok-spoorwegen-wfs" => ("rail", "CC0 1.0", PDOK),
        "pdok-bestuurlijk-wfs" => ("admin", "CC BY 4.0", PDOK),
        "pdok-natura2000-wfs" => ("protected", "CC0 1.0", PDOK),
        "vl-ortho-wms" => ("aerial", "Modellicentie gratis hergebruik", VL),
        "vl-grb-wfs" => ("buildings", "Modellicentie gratis hergebruik", VL),
        "osm-tiles" => ("basemap", "ODbL 1.0", &["EPSG:3857"]),
        "terrestris-osm-wms" => ("basemap", "ODbL 1.0", &["EPSG:3857", "EPSG:4326"]),
        "eox-s2-wms" => ("aerial", "CC BY-NC-SA 4.0", &["EPSG:3857", "EPSG:4326"]),
        "nasa-gibs-wms" => ("aerial", "public domain (NASA)", &["EPSG:4326"]),
        "gebco-wms" => ("elevation", "public domain (GEBCO)", &["EPSG:4326", "EPSG:3857"]),
        _ => ("", "", &[]),
    }
}

impl Service {
    fn finish(mut self) -> Self {
        let host = self.url.split("//").nth(1).and_then(|r| r.split('/').next()).unwrap_or("");
        self.hay = fold(&format!("{} | {} | {} | {} | {} | {}", self.name, self.provider, self.coverage, self.default_layer, host, self.id));
        self
    }

    /// Country or region name in the interface language.
    pub fn region(&self) -> &str {
        crate::lang::tr(&self.region_nl, &self.region_en)
    }

    pub fn covers(&self, p: LonLat) -> Option<bool> {
        self.bbox.map(|b| covers(b, p))
    }

    /// Area of the coverage box in square degrees (unknown = world).
    pub fn extent(&self) -> f64 {
        let b = self.bbox.unwrap_or(WORLD_BBOX);
        (b[2] - b[0]).abs() * (b[3] - b[1]).abs()
    }

    pub fn to_map_server(&self) -> MapServer {
        MapServer {
            id: self.id.clone(),
            kind: self.kind,
            name: self.name.clone(),
            region: self.region().to_string(),
            url: self.url.clone(),
            attribution: self.attribution.clone(),
            default_layer: self.default_layer.clone(),
            max_zoom: (self.kind == ServerKind::Xyz).then_some(19),
            builtin: self.source != Source::User,
        }
    }

    /// Match score for one query word, `None` when the word does not fit.
    fn word_score(&self, w: &str) -> Option<u32> {
        if let Some(k) = kind_for(w) {
            return (self.kind == k).then_some(1);
        }
        let cats = categories_for(w);
        let countries = countries_for(w);
        let name = fold(&self.name);
        let in_name = w.len() >= 3 && name.contains(w);
        if !cats.is_empty() {
            // A category word asks for the category (or a name that says it),
            // not for every service of a provider called "Kadaster".
            if cats.contains(&self.category.as_str()) {
                return Some(4 + in_name as u32);
            }
            if in_name {
                return Some(2);
            }
            if countries.is_empty() {
                return None;
            }
        }
        if !countries.is_empty() {
            if countries.contains(&self.country.as_str()) {
                return Some(3);
            }
            if w.len() >= 4 && fold(&self.coverage).contains(w) {
                return Some(3);
            }
            return None;
        }
        if w.len() <= 2 {
            return self.hay.split(|c: char| !c.is_alphanumeric()).any(|t| t == w).then_some(1);
        }
        let coverage = fold(&self.coverage);
        if PLACE_WORDS.iter().any(|(alias, en)| *alias == w && coverage.contains(en)) {
            return Some(3);
        }
        if in_name {
            return Some(3);
        }
        if fold(&self.region_nl).contains(w) || fold(&self.region_en).contains(w) || fold(&self.coverage).contains(w) {
            return Some(2);
        }
        if self.hay.contains(w) || self.crs.iter().any(|c| fold(c).contains(w)) {
            return Some(1);
        }
        None
    }

    /// Search result as JSON, for the command and the MCP server.
    pub fn json(&self, at: Option<LonLat>) -> Value {
        json!({
            "id": self.id,
            "type": match self.kind { ServerKind::Wms => "WMS", ServerKind::Wfs => "WFS", ServerKind::Xyz => "XYZ" },
            "name": self.name,
            "provider": self.provider,
            "country": self.country,
            "region": self.region_en,
            "coverage": self.coverage,
            "category": self.category,
            "url": self.url,
            "default_layer": self.default_layer,
            "crs": self.crs,
            "license": self.license,
            "terms": self.terms,
            "bbox": self.bbox,
            "covers_location": at.and_then(|p| self.covers(p)),
        })
    }
}

pub fn covers(b: [f64; 4], p: LonLat) -> bool {
    p[0] >= b[0] && p[0] <= b[2] && p[1] >= b[1] && p[1] <= b[3]
}

/// All services: built-in presets, the worldwide list, the user's servers.
pub fn all() -> Vec<Service> {
    let mut out = Vec::new();
    for s in servers::builtin() {
        let (cat, license, crs) = builtin_meta(&s.id);
        let (country, nl, en, bbox) = if s.id.starts_with("pdok") {
            ("NL", "Nederland", "Netherlands", NL_BBOX)
        } else if s.id.starts_with("vl-") {
            ("BE", "België", "Belgium", FLANDERS_BBOX)
        } else {
            ("GLOBAL", "Wereld", "World", WORLD_BBOX)
        };
        let provider = s.attribution.split(['(', '©']).map(str::trim).find(|p| !p.is_empty()).unwrap_or("").to_string();
        out.push(Service {
            id: s.id.clone(),
            kind: s.kind,
            name: s.name.clone(),
            provider,
            country: country.into(),
            region_nl: nl.into(),
            region_en: en.into(),
            coverage: if country == "BE" { "Flanders, Belgium".into() } else { en.into() },
            category: cat.into(),
            url: s.url.clone(),
            default_layer: s.default_layer.clone(),
            crs: crs.iter().map(|c| c.to_string()).collect(),
            license: license.into(),
            terms: String::new(),
            attribution: s.attribution.clone(),
            bbox: Some(bbox),
            source: Source::Builtin,
            hay: String::new(),
        }.finish());
    }
    for w in servers::world() {
        out.push(Service {
            id: w.id.clone(),
            kind: w.kind,
            name: w.name.clone(),
            provider: w.provider.clone(),
            country: w.country.clone(),
            region_nl: w.region_nl.clone(),
            region_en: w.region_en.clone(),
            coverage: w.coverage.clone(),
            category: w.category.clone(),
            url: w.url.clone(),
            default_layer: w.default_layer.clone(),
            crs: if w.crs.is_empty() && w.kind == ServerKind::Xyz { vec!["EPSG:3857".into()] } else { w.crs.clone() },
            license: w.license.clone(),
            terms: w.terms.clone(),
            attribution: w.attribution.clone(),
            bbox: w.bbox,
            source: Source::World,
            hay: String::new(),
        }.finish());
    }
    for u in servers::load_user() {
        out.push(user_service(&u));
    }
    out
}

/// A user server (or any URL) as a catalogue entry: coverage unknown.
pub fn user_service(u: &MapServer) -> Service {
    Service {
        id: u.id.clone(),
        kind: u.kind,
        name: u.name.clone(),
        provider: u.attribution.clone(),
        country: String::new(),
        region_nl: if u.region.is_empty() { "Eigen servers".into() } else { u.region.clone() },
        region_en: if u.region.is_empty() { "My servers".into() } else { u.region.clone() },
        coverage: String::new(),
        category: String::new(),
        url: u.url.clone(),
        default_layer: u.default_layer.clone(),
        crs: if u.kind == ServerKind::Xyz { vec!["EPSG:3857".into()] } else { Vec::new() },
        license: String::new(),
        terms: String::new(),
        attribution: u.attribution.clone(),
        bbox: None,
        source: Source::User,
        hay: String::new(),
    }
    .finish()
}

/// What to look for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Query {
    /// ISO region codes; empty = all. Include GLOBAL explicitly when desired.
    pub countries: Vec<String>,
    pub text: String,
    /// Category ids; empty = all.
    pub categories: Vec<String>,
    /// Service types; empty = all.
    pub kinds: Vec<ServerKind>,
    /// Only services whose coverage contains this point (lon, lat). Services
    /// with unknown coverage (user servers) stay in.
    pub at: Option<LonLat>,
}

/// Indices into `services` of the matches, best first: the highest score,
/// then (with a location) the smallest coverage, so local services come
/// before national and worldwide ones.
pub fn search(services: &[Service], q: &Query) -> Vec<usize> {
    let ws = words(&q.text);
    let mut hits: Vec<(usize, u32)> = services
        .iter()
        .enumerate()
        .filter(|(_,s)|q.countries.is_empty()||q.countries.iter().any(|c|c.eq_ignore_ascii_case(&s.country)))
        .filter(|(_, s)| q.kinds.is_empty() || q.kinds.contains(&s.kind))
        .filter(|(_, s)| q.categories.is_empty() || q.categories.iter().any(|c| c == &s.category))
        .filter(|(_, s)| q.at.is_none_or(|p| s.covers(p) != Some(false)))
        .filter_map(|(i, s)| {
            let mut score = 0;
            for w in &ws {
                score += s.word_score(w)?;
            }
            Some((i, score))
        })
        .collect();
    hits.sort_by(|a, b| {
        b.1.cmp(&a.1).then_with(|| {
            if q.at.is_some() {
                services[a.0].extent().partial_cmp(&services[b.0].extent()).unwrap_or(std::cmp::Ordering::Equal)
            } else {
                std::cmp::Ordering::Equal
            }
        })
    });
    hits.into_iter().map(|(i, _)| i).collect()
}

/// Matches grouped by country or region, in order of their best match.
pub fn group(services: &[Service], hits: &[usize]) -> Vec<(String, Vec<usize>)> {
    let mut out: Vec<(String, Vec<usize>)> = Vec::new();
    for &i in hits {
        let key = services[i].region().to_string();
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => v.push(i),
            None => out.push((key, vec![i])),
        }
    }
    out
}

/// Coordinate systems of a service against the project CRS: what will be
/// requested and whether it transforms.
#[derive(Clone, Debug, PartialEq)]
pub struct CrsPlan {
    /// The system the request will use.
    pub request: String,
    /// Same as the project CRS: no reprojection.
    pub native: bool,
    pub support: Support,
}

/// The CRS the existing import paths will request for `offered` (as the
/// service or layer lists them) in a project in `site_crs`.
pub fn crs_plan(kind: ServerKind, offered: &[String], site_crs: &str) -> CrsPlan {
    let offered: Vec<String> = offered.iter().map(|c| crs::normalize(c)).collect();
    let request = match kind {
        ServerKind::Xyz => "EPSG:3857".to_string(),
        ServerKind::Wms => {
            let l = super::caps::WmsLayer { name: String::new(), title: String::new(), crs: offered, bbox: None };
            super::raster::request_crs(&l, site_crs)
        }
        ServerKind::Wfs => {
            let t = super::caps::WfsType { name: String::new(), title: String::new(), crs: offered, bbox: None };
            super::wfs::request_crs(&t, site_crs)
        }
    };
    let native = request == site_crs;
    let support = if native { Support::Builtin } else { crs::support(&request) };
    CrsPlan { request, native, support }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn svc(id: &str, kind: ServerKind, country: &str, category: &str, name: &str, coverage: &str, bbox: [f64; 4]) -> Service {
        Service {
            id: id.into(),
            kind,
            name: name.into(),
            provider: "Kadaster".into(),
            country: country.into(),
            region_nl: country.into(),
            region_en: country.into(),
            coverage: coverage.into(),
            category: category.into(),
            url: "https://example.org/wms".into(),
            default_layer: String::new(),
            crs: vec!["EPSG:25832".into()],
            license: String::new(),
            terms: String::new(),
            attribution: String::new(),
            bbox: Some(bbox),
            source: Source::World,
            hay: String::new(),
        }
        .finish()
    }

    fn sample() -> Vec<Service> {
        vec![
            svc("by-flur", ServerKind::Wms, "DE", "cadastre", "ALKIS Flurstücke Bayern", "Bavaria, Germany", [8.97, 47.27, 13.84, 50.57]),
            svc("nrw-flur", ServerKind::Wfs, "DE", "cadastre", "ALKIS NRW", "North Rhine-Westphalia, Germany", [5.87, 50.32, 9.46, 52.53]),
            svc("nl-bag", ServerKind::Wfs, "NL", "buildings", "BAG panden", "Netherlands", [3.2, 50.75, 7.23, 53.6]),
            svc("nl-brk", ServerKind::Wms, "NL", "cadastre", "Kadastrale kaart", "Netherlands", [3.2, 50.75, 7.23, 53.6]),
            svc("world-s2", ServerKind::Xyz, "GLOBAL", "aerial", "Sentinel-2 cloudless", "Worldwide", WORLD_BBOX),
            svc("de-dop", ServerKind::Wms, "DE", "aerial", "Digitale Orthophotos", "Germany", [5.87, 47.27, 15.04, 55.06]),
        ]
    }

    #[test]
    fn region_filter_keeps_global_sources_but_excludes_other_countries() {
        let services=sample();
        let query=Query{countries:vec!["NL".into(),"GLOBAL".into()],..Default::default()};
        let result=ids(&services,&query);
        assert!(result.contains(&"nl-bag".into()));assert!(result.contains(&"world-s2".into()));assert!(!result.contains(&"by-flur".into()));
        let us:Vec<_>=all().into_iter().filter(|s|s.country=="US").collect();
        assert!(us.len()>=20);
    }

    fn ids(s: &[Service], q: &Query) -> Vec<String> {
        search(s, q).into_iter().map(|i| s[i].id.clone()).collect()
    }

    #[test]
    fn folds_and_splits() {
        assert_eq!(fold("Flurstücke Baden-Württemberg ß"), "flurstucke baden-wurttemberg ss");
        assert_eq!(words("percelen in Duitsland"), vec!["percelen", "duitsland"]);
        assert_eq!(words("de"), vec!["de"]);
        assert_eq!(categories_for("flurstucke"), vec!["cadastre"]);
        assert_eq!(categories_for("luchtfoto"), vec!["aerial"]);
        assert_eq!(categories_for("dem"), vec!["elevation"]);
    }

    #[test]
    fn category_words_in_any_language() {
        let s = sample();
        for q in ["kadaster", "cadastre", "parcels", "percelen", "Flurstücke", "cadastro"] {
            let got = ids(&s, &Query { text: q.into(), ..Default::default() });
            assert_eq!(got.len(), 3, "{q}: {got:?}");
            assert!(!got.contains(&"nl-bag".to_string()), "{q}: provider Kadaster is not a cadastre service");
        }
        assert_eq!(ids(&s, &Query { text: "luchtfoto".into(), ..Default::default() }).len(), 2);
    }

    #[test]
    fn country_and_place_words() {
        let s = sample();
        assert_eq!(ids(&s, &Query { text: "kadaster duitsland".into(), ..Default::default() }), vec!["by-flur", "nrw-flur"]);
        assert_eq!(ids(&s, &Query { text: "percelen Bayern".into(), ..Default::default() }), vec!["by-flur"]);
        assert_eq!(ids(&s, &Query { text: "parcels netherlands".into(), ..Default::default() }), vec!["nl-brk"]);
        assert_eq!(ids(&s, &Query { text: "sentinel".into(), ..Default::default() }), vec!["world-s2"]);
        assert!(ids(&s, &Query { text: "nonsenseword".into(), ..Default::default() }).is_empty());
    }

    #[test]
    fn filters_and_location() {
        let s = sample();
        let munich = [11.58, 48.14];
        let q = Query { at: Some(munich), ..Default::default() };
        // Local first: Bavaria, then Germany, then the world.
        assert_eq!(ids(&s, &q), vec!["by-flur", "de-dop", "world-s2"]);
        let q = Query { text: "aerial".into(), at: Some([4.66, 51.81]), ..Default::default() };
        assert_eq!(ids(&s, &q), vec!["world-s2"]);
        let q = Query { kinds: vec![ServerKind::Wfs], ..Default::default() };
        assert_eq!(ids(&s, &q), vec!["nrw-flur", "nl-bag"]);
        let q = Query { categories: vec!["cadastre".into()], at: Some([4.66, 51.81]), ..Default::default() };
        assert_eq!(ids(&s, &q), vec!["nl-brk"]);
        let groups = group(&s, &search(&s, &Query { text: "cadastre".into(), ..Default::default() }));
        assert_eq!(groups.iter().map(|g| g.1.len()).collect::<Vec<_>>(), vec![2, 1]);
    }

    #[test]
    fn catalogue_has_coverage_and_crs() {
        let all = all();
        let world: Vec<&Service> = all.iter().filter(|s| s.source == Source::World).collect();
        assert!(world.len() >= 600, "{}", world.len());
        assert!(world.iter().all(|s| s.bbox.is_some() && !s.crs.is_empty()), "every worldwide service has a box and a CRS");
        // Cadastral parcels at a few places around the world.
        for (place, p) in [("Munich", [11.58, 48.14]), ("Sydney", [151.2, -33.87]), ("Dordrecht", [4.66, 51.81]), ("Vienna", [16.37, 48.21])] {
            let q = Query { text: "cadastral parcels".into(), at: Some(p), ..Default::default() };
            assert!(!search(&all, &q).is_empty(), "no cadastre at {place}");
        }
        let presets: Vec<&str> = all.iter().filter(|s| s.source != Source::User).map(|s| s.id.as_str()).collect();
        let unique: std::collections::HashSet<&str> = presets.iter().copied().collect();
        assert_eq!(unique.len(), presets.len(), "preset ids are unique");
    }

    /// Online: GetCapabilities of one WMS or WFS in each of a dozen countries
    /// parses with the existing reader. `cargo test --release -p open-infra --lib capabilities_online -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn capabilities_online() {
        let all = all();
        let mut countries = std::collections::HashSet::new();
        let picks: Vec<&Service> = all
            .iter()
            .filter(|s| s.source == Source::World && s.kind != ServerKind::Xyz && !["GLOBAL", "EU", "AFRICA"].contains(&s.country.as_str()))
            .filter(|s| countries.insert(s.country.clone()))
            .step_by(4)
            .take(12)
            .collect();
        let mut ok = 0;
        for s in &picks {
            let r = match s.kind {
                ServerKind::Wms => super::super::caps::fetch_wms(&s.url).map(|c| format!("{} layers, {} with extent", c.layers.len(), c.layers.iter().filter(|l| l.bbox.is_some()).count())),
                _ => super::super::caps::fetch_wfs(&s.url).map(|c| format!("{} types, {} with extent", c.types.len(), c.types.iter().filter(|t| t.bbox.is_some()).count())),
            };
            println!("{:6} {:4} {}: {}", s.country, s.kind.label(), s.id, r.as_ref().map(String::as_str).unwrap_or_else(|e| e.as_str()));
            ok += r.is_ok() as usize;
        }
        assert!(ok * 10 >= picks.len() * 7, "{ok} of {} parsed", picks.len());
    }

    #[test]
    fn crs_plans() {
        let v = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let p = crs_plan(ServerKind::Wms, &v(&["EPSG:25832", "EPSG:3857"]), "EPSG:28992");
        assert_eq!((p.request.as_str(), p.native, p.support), ("EPSG:3857", false, Support::Builtin));
        let p = crs_plan(ServerKind::Wfs, &v(&["urn:ogc:def:crs:EPSG::28992"]), "EPSG:28992");
        assert!(p.native);
        let p = crs_plan(ServerKind::Wms, &v(&["EPSG:2180"]), "EPSG:28992");
        assert_eq!(p.support, Support::Online);
        let p = crs_plan(ServerKind::Wms, &v(&["IGNF:LAMB93"]), "EPSG:28992");
        assert_eq!(p.support, Support::Unsupported);
        assert_eq!(crs_plan(ServerKind::Xyz, &[], "EPSG:28992").request, "EPSG:3857");
    }
}
