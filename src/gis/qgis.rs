//! Connections exported from QGIS (Data Source Manager ▸ Save connections):
//! `qgsWMSConnections`, `qgsWFSConnections` and `qgsXYZTilesConnections`.
//!
//! The XML is read without QGIS and without network. Settings Open Infra
//! cannot honour (authentication, own HTTP headers, a different axis order,
//! placeholders the tile client does not know) make a connection
//! non-importable instead of importing it as if it worked. Secrets are never
//! copied, logged or shown: a message names the setting, not its value.

use std::path::Path;

use crate::lang::tr;

use super::http::clean_service_url;
use super::servers::{MapServer, ServerKind};

/// One connection from the export, with what stands in the way of importing it.
#[derive(Clone, Debug)]
pub struct Connection {
    pub server: MapServer,
    /// Settings Open Infra cannot honour; any of them blocks the import.
    pub blocking: Vec<String>,
    /// Already among the user's servers or the built-in presets, or earlier in this file.
    pub duplicate: bool,
}

impl Connection {
    pub fn importable(&self) -> bool {
        self.blocking.is_empty() && !self.duplicate
    }
}

/// Reads a QGIS connections export from disk.
pub fn read(path: &Path) -> Result<Vec<Connection>, String> {
    let xml = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&xml)
}

/// Parses a QGIS connections export (WMS, WFS or XYZ).
pub fn parse(xml: &str) -> Result<Vec<Connection>, String> {
    // QGIS writes headers as `http-header:<name>` attributes without declaring
    // that prefix, which a namespace-aware parser rejects.
    let xml = [' ', '\n', '\r', '\t'].iter().fold(xml.to_string(), |x, c| x.replace(&format!("{c}http-header:"), &format!("{c}http-header_")));
    let opt = roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let doc = roxmltree::Document::parse_with_options(&xml, opt)
        .map_err(|_| tr("Geen geldige XML", "Not valid XML").to_string())?;
    let root = doc.root_element();
    let (kind, element) = match root.tag_name().name() {
        "qgsWMSConnections" => (ServerKind::Wms, "wms"),
        "qgsWFSConnections" => (ServerKind::Wfs, "wfs"),
        "qgsXYZTilesConnections" => (ServerKind::Xyz, "xyztiles"),
        other => {
            return Err(format!(
                "{} <{other}>. {}",
                tr("Geen QGIS-export van WMS-, WFS- of XYZ-verbindingen:", "Not a QGIS export of WMS, WFS or XYZ connections:"),
                tr("Exporteer in QGIS via Databronbeheer ▸ Verbindingen opslaan.", "Export in QGIS via Data Source Manager ▸ Save connections."),
            ))
        }
    };
    let mut out: Vec<Connection> = Vec::new();
    for (i, n) in root.children().filter(|n| n.is_element() && n.has_tag_name(element)).enumerate() {
        let attr = |k: &str| n.attribute(k).unwrap_or("").trim();
        let flag = |k: &str| matches!(attr(k), "1" | "true");
        let raw_url = attr("url");
        let url_auth = url_needs_auth(raw_url);
        let mut blocking = Vec::new();
        if !raw_url.split_once("://").is_some_and(|(scheme, _)| scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
            blocking.push(tr("geen http(s)-adres", "not an http(s) address").to_string());
        }
        if url_auth || !attr("username").is_empty() || !attr("password").is_empty() || !attr("authcfg").is_empty() {
            blocking.push(tr("vraagt authenticatie", "needs authentication").to_string());
        }
        let headers = n.attributes().any(|a| a.name().starts_with("http-header_") && !a.value().trim().is_empty());
        if headers || !attr("referer").is_empty() {
            blocking.push(tr("eigen HTTP-headers of referer", "own HTTP headers or referer").to_string());
        }
        if kind != ServerKind::Xyz && (flag("invertAxisOrientation") || flag("ignoreAxisOrientation")) {
            blocking.push(tr("afwijkende asvolgorde", "different axis order").to_string());
        }
        let mut server = MapServer {
            id: String::new(),
            kind,
            name: String::new(),
            region: "QGIS".into(),
            url: if url_auth { String::new() } else if kind == ServerKind::Xyz { raw_url.to_string() } else { clean_service_url(raw_url) },
            attribution: String::new(),
            default_layer: String::new(),
            max_zoom: None,
            min_zoom: None,
            builtin: false,
        };
        if kind == ServerKind::Xyz {
            if !url_auth {
                blocking.extend(xyz_problems(raw_url));
            }
            if !matches!(attr("tilePixelRatio"), "" | "0" | "1") {
                blocking.push(tr("tegels van hoge resolutie (512 px)", "high-resolution tiles (512 px)").to_string());
            }
            if !attr("interpretation").is_empty() {
                blocking.push(tr("hoogtetegels (geen kaartbeeld)", "elevation tiles (not a map image)").to_string());
            }
            let zoom = |k: &str| attr(k).parse::<u32>().ok().filter(|z| *z <= 30);
            server.max_zoom = Some(zoom("zmax").unwrap_or(19));
            server.min_zoom = zoom("zmin").filter(|z| *z > 0);
            if server.min_zoom > server.max_zoom {
                blocking.push(tr("minimumzoom boven maximumzoom", "minimum zoom above maximum zoom").to_string());
            }
        }
        server.name = match attr("name") {
            "" => host(&server.url).unwrap_or_else(|| format!("{} {}", kind.label(), i + 1)),
            name => name.to_string(),
        };
        let duplicate = blocking.is_empty() && out.iter().any(|c| c.importable() && same(&c.server, &server));
        out.push(Connection { server, blocking, duplicate });
    }
    if out.is_empty() {
        return Err(tr("Het bestand bevat geen verbindingen.", "The file contains no connections.").into());
    }
    Ok(out)
}

/// Credentials in userinfo or query parameters are not supported. Decode
/// parameter names only: query values retain their original meaning.
fn url_needs_auth(url: &str) -> bool {
    let raw_authority = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url).split(['/', '?', '#']).next().unwrap_or("");
    let authority = decode_url_component(raw_authority).unwrap_or_default();
    if raw_authority.contains('@') || authority.contains('@') {
        return true;
    }
    let host = authority.split(':').next().unwrap_or("").trim_end_matches('.').to_ascii_lowercase();
    let in_domain = |domain: &str| host == domain || host.strip_suffix(domain).is_some_and(|prefix| prefix.ends_with('.'));
    // `key` can be a public selector (for example, an OSM tag in Taginfo).
    // Only the documented map API hosts give it an authentication role.
    let aws_maps = ["maps.geo.", "maps.geo-fips."].iter().any(|&prefix| {
        host.strip_prefix(prefix).is_some_and(|rest| [".amazonaws.com", ".api.aws"].iter().any(|&suffix| {
            rest.strip_suffix(suffix).is_some_and(|region| !region.is_empty() && region.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        }))
    });
    let key_auth = matches!(host.as_str(), "api.maptiler.com" | "maps.googleapis.com" | "tile.googleapis.com" | "dev.ditu.live.com")
        || ["api.tomtom.com", "basemaps.cartocdn.com", "virtualearth.net"].iter().any(|&domain| in_domain(domain)) || aws_maps;
    // OpenWeather calls its API key `appid`; HERE's app_id is an identifier.
    let appid_auth = in_domain("openweathermap.org");
    let Some((_, query)) = url.split_once('?') else { return false };
    query.split(['&', ';']).any(|parameter| {
        let Some(decoded) = decode_url_component(parameter.split('=').next().unwrap_or("")) else { return false };
        let key: String = decoded.chars().filter(|&c| !matches!(c, '_' | '-')).map(|c| c.to_ascii_lowercase()).collect();
        matches!(key.as_str(), "user" | "username" | "password" | "passwd" | "pwd" | "auth" | "authcfg" | "authorization"
            | "authkey" | "appcode" | "jwt" | "sessionid" | "jsessionid" | "phpsessid" | "asp.netsessionid"
            | "accesskey" | "rapidapikey" | "xrapidapikey" | "oauthtoken" | "oauthtokensecret" | "oauthsignature"
            | "token" | "accesstoken" | "refreshtoken" | "idtoken" | "apikey" | "xapikey" | "subscriptionkey" | "secret" | "clientsecret"
            | "signature" | "sig" | "awsaccesskeyid" | "xamzcredential" | "xamzsecuritytoken" | "xamzsignature" | "xgoogcredential" | "xgoogsignature")
            || (key == "key" && key_auth) || (key == "appid" && appid_auth)
    })
}

/// Decode names/authority once; never decode or rewrite the query values.
fn decode_url_component(value: &str) -> Option<String> {
    let mut bytes = value.bytes();
    let mut out = Vec::with_capacity(value.len());
    while let Some(b) = bytes.next() {
        let b = if b == b'%' {
            let hi = (bytes.next()? as char).to_digit(16)?;
            let lo = (bytes.next()? as char).to_digit(16)?;
            (hi * 16 + lo) as u8
        } else { b };
        out.push(b);
    }
    String::from_utf8(out).ok()
}

/// Placeholders the tile client fills in; anything else in braces is not supported.
fn xyz_problems(url: &str) -> Vec<String> {
    let mut out = Vec::new();
    let has_y = url.contains("{y}") || url.contains("{-y}");
    if !(url.contains("{q}") || (url.contains("{z}") && url.contains("{x}") && has_y)) {
        out.push(tr("tegel-URL zonder {z}, {x} en {y}", "tile URL without {z}, {x} and {y}").to_string());
    }
    let mut rest = url;
    while let Some(start) = rest.find('{') {
        let Some(len) = rest[start..].find('}') else { break };
        let token = &rest[start..start + len + 1];
        if !matches!(token, "{z}" | "{x}" | "{y}" | "{-y}" | "{q}") {
            out.push(format!("{} {token}", tr("onbekende plaatshouder", "unknown placeholder")));
        }
        rest = &rest[start + len + 1..];
    }
    out
}

fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', ':']).next()?;
    (!host.is_empty()).then(|| host.to_string())
}

/// The same service: kind and address, ignoring case of scheme and host,
/// a trailing path slash and an empty query. Query values stay unchanged.
fn same(a: &MapServer, b: &MapServer) -> bool {
    a.kind == b.kind && key(&a.url) == key(&b.url)
}

fn key(url: &str) -> String {
    let (path, query) = url.trim().split_once('?').unwrap_or((url.trim(), ""));
    let path = path.trim_end_matches('/');
    let mut key = match path.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
            format!("{}://{}{path}", scheme.to_ascii_lowercase(), host.to_ascii_lowercase())
        }
        None => path.to_string(),
    };
    if !query.is_empty() {
        key.push('?');
        key.push_str(query);
    }
    key
}

/// Marks connections already present among `existing` (user servers and presets).
pub fn mark_duplicates(conns: &mut [Connection], existing: &[MapServer]) {
    for c in conns.iter_mut() {
        if c.blocking.is_empty() && existing.iter().any(|e| same(e, &c.server)) {
            c.duplicate = true;
        }
    }
}

/// The user's servers with the chosen connections added (non-importable and
/// duplicate ones are skipped). Returns the new list and how many were added.
pub fn merge(user: &[MapServer], chosen: &[&Connection], stamp: u64) -> (Vec<MapServer>, usize) {
    let mut out = user.to_vec();
    let mut added = 0;
    let mut next = 0;
    for c in chosen.iter().filter(|c| c.blocking.is_empty() && !url_needs_auth(&c.server.url)) {
        if out.iter().any(|e| same(e, &c.server)) {
            continue;
        }
        let mut s = c.server.clone();
        loop {
            s.id = format!("qgis-{stamp}-{next}");
            next += 1;
            if !out.iter().any(|e| e.id.eq_ignore_ascii_case(&s.id)) {
                break;
            }
        }
        out.push(s);
        added += 1;
    }
    (out, added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{}/tests/fixtures/qgis/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
    }

    #[test]
    fn wms_export_with_entities_and_unsupported_settings() {
        let c = parse(&fixture("wms.xml")).unwrap();
        assert_eq!(c.len(), 4);
        assert_eq!(c[0].server.kind, ServerKind::Wms);
        assert_eq!(c[0].server.name, "PDOK Luchtfoto");
        assert_eq!(c[0].server.url, "https://service.pdok.nl/hwh/luchtfotorgb/wms/v1_0");
        assert!(c[0].importable());
        // &amp; in the attribute is an ampersand; GetCapabilities parameters go.
        assert_eq!(c[1].server.url, "https://example.org/cgi-bin/mapserv?map=/data/a.map&lang=nl");
        assert!(c[1].importable());
        assert!(!c[2].importable());
        assert!(c[2].blocking.iter().any(|b| b.contains("authenticat")));
        assert!(c[3].blocking.iter().any(|b| b.contains("axis") || b.contains("asvolgorde")));
    }

    #[test]
    fn secrets_never_reach_the_server_or_the_messages() {
        let c = parse(&fixture("wms.xml")).unwrap();
        let text = format!("{c:?}");
        assert!(!text.contains("geheim-wachtwoord"));
        assert!(!text.contains("jan.jansen"));
    }

    #[test]
    fn wfs_export() {
        let c = parse(&fixture("wfs.xml")).unwrap();
        assert_eq!(c.len(), 2);
        assert!(c.iter().all(|c| c.server.kind == ServerKind::Wfs));
        assert_eq!(c[0].server.url, "https://service.pdok.nl/kadaster/kadastralekaart/wfs/v5_0");
        assert!(c[0].importable());
        assert!(c[1].blocking.iter().any(|b| b.contains("headers")));
    }

    #[test]
    fn xyz_export_with_zoom_limits_and_placeholders() {
        let c = parse(&fixture("xyz.xml")).unwrap();
        assert_eq!(c.len(), 6);
        assert_eq!((c[0].server.min_zoom, c[0].server.max_zoom), (None, Some(19)));
        assert!(c[0].importable());
        assert_eq!((c[1].server.min_zoom, c[1].server.max_zoom), (Some(6), Some(16)));
        assert!(c[1].importable(), "{{-y}} is supported");
        assert!(c[2].blocking.iter().any(|b| b.contains("{s}")));
        assert!(!c[3].importable(), "512 px tiles");
        assert!(!c[4].importable(), "elevation tiles");
        assert!(c[5].duplicate, "same URL as the first, other case and a trailing slash");
    }

    #[test]
    fn duplicates_against_existing_servers_and_repeated_import() {
        let mut c = parse(&fixture("xyz.xml")).unwrap();
        let existing = vec![c[1].server.clone()];
        mark_duplicates(&mut c, &existing);
        assert!(c[1].duplicate && !c[0].duplicate);
        let chosen: Vec<&Connection> = c.iter().collect();
        let (once, added) = merge(&[], &chosen, 7);
        assert_eq!(added, 2, "the first and second; the rest is blocked or a duplicate in the file");
        let (twice, again) = merge(&once, &chosen, 8);
        assert_eq!((again, twice.len()), (0, once.len()));
        assert!(once.iter().all(|s| s.id.starts_with("qgis-7-")));
    }

    #[test]
    fn invalid_or_foreign_xml_is_an_error() {
        assert!(parse(&fixture("kapot.xml")).is_err());
        assert!(parse("<qgsArcGisMapServerConnections/>").unwrap_err().contains("qgsArcGisMapServerConnections"));
        assert!(parse("<qgsWMSConnections version=\"1.0\"/>").is_err());
    }

    #[test]
    fn tile_url_placeholders() {
        use crate::gis::raster::tile_url;
        assert_eq!(tile_url("https://t/{z}/{x}/{y}.png", 3, 4, 2), "https://t/3/4/2.png");
        assert_eq!(tile_url("https://t/{z}/{x}/{-y}.png", 3, 4, 2), "https://t/3/4/5.png");
        // Bing's documentation example: tile 3,5 at level 3 is quadkey 213.
        assert_eq!(tile_url("https://t/{q}.jpeg", 3, 3, 5), "https://t/213.jpeg");
    }

    #[test]
    fn url_credentials_are_blocked_and_not_kept_for_preview() {
        let mut failures = Vec::new();
        for (root, element, suffix) in [("qgsWMSConnections", "wms", "/wms"), ("qgsWFSConnections", "wfs", "/wfs"), ("qgsXYZTilesConnections", "xyztiles", "/{z}/{x}/{y}.png")] {
            for credential in ["userinfo", "username", "password", "token", "apikey", "api_key", "access_token", "authcfg", "authorization", "PASS%57ORD", "X-Api-Key", "client_secret"] {
                let url = if credential == "userinfo" {
                    format!("https://user:SYNTHETIC_VALUE@example.invalid{suffix}")
                } else {
                    format!("https://example.invalid{suffix}?{credential}=SYNTHETIC_VALUE")
                };
                let xml = format!("<{root}><{element} url=\"{url}\"/></{root}>");
                let connections = parse(&xml).unwrap();
                let c = &connections[0];
                if c.importable() || format!("{c:?}").contains("SYNTHETIC_VALUE") || c.server.name == "user" {
                    failures.push(format!("{element}/{credential}"));
                }
            }
        }
        assert!(failures.is_empty(), "Niet veilig geblokkeerd: {}", failures.join(", "));
    }

    #[test]
    fn url_credentials_do_not_enter_the_saved_server_list() {
        let connections = parse(r#"<qgsWMSConnections><wms name="Besloten" url="https://example.invalid/wms?username=user&amp;password=SYNTHETIC_VALUE"/></qgsWMSConnections>"#).unwrap();
        let chosen: Vec<_> = connections.iter().collect();
        let (servers, added) = merge(&[], &chosen, 888);
        assert_eq!(added, 0);
        assert!(!serde_json::to_string(&servers).unwrap().contains("SYNTHETIC_VALUE"));
    }

    #[test]
    fn azure_maps_subscription_keys_are_blocked_and_discarded() {
        let connections = parse(&fixture("azure-maps.xml")).unwrap();
        assert_eq!(connections.len(), 3);
        let mut failures = Vec::new();
        for c in &connections {
            let (servers, added) = merge(&[], &[c], 888);
            let blocked = c.blocking.iter().any(|reason| reason == tr("vraagt authenticatie", "needs authentication"));
            println!("{}: importeerbaar={}, authenticatieblokkade={blocked}, lege URL={}, toegevoegd={added}", c.server.name, c.importable(), c.server.url.is_empty());
            if c.importable() || !blocked || !c.server.url.is_empty() || added != 0 || !servers.is_empty() {
                failures.push(c.server.name.clone());
            }
        }
        assert!(failures.is_empty(), "Niet veilig geblokkeerd: {}", failures.join(", "));
    }

    #[test]
    fn a_blocked_connection_does_not_disable_the_public_connection() {
        let connections = parse(r#"<qgsWMSConnections>
            <wms name="Besloten" url="https://example.invalid/wms" username="user"/>
            <wms name="Openbaar" url="https://example.invalid/wms"/>
        </qgsWMSConnections>"#).unwrap();
        assert!(!connections[0].importable());
        assert!(connections[1].importable());
        let (servers, added) = merge(&[], &[&connections[1]], 888);
        assert_eq!((servers.len(), added), (1, 1));
    }

    #[test]
    fn duplicate_detection_preserves_query_values() {
        let mut connections = parse(r#"<qgsWMSConnections>
            <wms name="Zonder slash" url="https://example.invalid/wms?map=/data/a"/>
            <wms name="Met slash" url="https://example.invalid/wms?map=/data/a/"/>
            <wms name="Met vraagteken" url="https://example.invalid/wms?map=/data/a?"/>
        </qgsWMSConnections>"#).unwrap();
        assert!(connections.iter().all(Connection::importable));
        let existing = vec![connections[0].server.clone()];
        mark_duplicates(&mut connections, &existing);
        assert!(connections[0].duplicate);
        assert!(connections[1].importable() && connections[2].importable());
        let (servers, added) = merge(&[], &connections.iter().collect::<Vec<_>>(), 888);
        assert_eq!((servers.len(), added), (3, 3));
    }

    #[test]
    fn two_imports_in_one_second_have_unique_ids() {
        let a = parse(r#"<qgsWMSConnections><wms name="A" url="https://a.invalid/wms"/></qgsWMSConnections>"#).unwrap();
        let b = parse(r#"<qgsWMSConnections><wms name="B" url="https://b.invalid/wms"/></qgsWMSConnections>"#).unwrap();
        let (mut user, _) = merge(&[], &[&a[0]], 888);
        user[0].id.make_ascii_uppercase();
        let (user, added) = merge(&user, &[&b[0]], 888);
        assert_eq!(added, 1);
        assert_ne!(user[0].id.to_ascii_lowercase(), user[1].id.to_ascii_lowercase());
        assert_eq!(user[1].url, b[0].server.url);
    }

    #[test]
    fn xml_parser_errors_do_not_echo_input() {
        let error = parse(r#"<qgsWMSConnections><wms password="&SYNTHETIC_VALUE;"/></qgsWMSConnections>"#).unwrap_err();
        assert!(!error.contains("SYNTHETIC_VALUE"));
        assert_eq!(error, tr("Geen geldige XML", "Not valid XML"));
    }
}

#[cfg(test)]
#[path = "../../tests/support/qgis_auth.rs"]
pub(crate) mod auth_test_support;
