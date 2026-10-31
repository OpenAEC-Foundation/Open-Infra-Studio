//! Blocking HTTP for the GIS services (runs on worker threads, never on the
//! host's dispatch path).

use std::sync::OnceLock;
use std::time::Duration;

use ureq::Agent;

const USER_AGENT: &str = concat!(
    "OpenInfra/",
    env!("CARGO_PKG_VERSION"),
    " (Open CAD Studio plugin; https://github.com/OpenAEC-Foundation/Open-Infra)"
);
const MAX_BODY: u64 = 256 * 1024 * 1024;

fn agent(timeout_s: u64) -> Agent {
    Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_s)))
        .user_agent(USER_AGENT)
        .http_status_as_error(false)
        .build()
        .into()
}

fn default_agent() -> &'static Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT.get_or_init(|| agent(120))
}

pub struct Fetched {
    pub content_type: String,
    pub body: Vec<u8>,
}

fn host_of(url: &str) -> &str {
    url.split("://").nth(1).and_then(|r| r.split('/').next()).unwrap_or(url)
}

fn finish(url: &str, resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Fetched, String> {
    let mut resp = resp.map_err(|e| format!("{}: {e}", host_of(url)))?;
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let body = resp
        .body_mut()
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .map_err(|e| format!("{}: {e}", host_of(url)))?;
    if !(200..300).contains(&status) {
        let text = String::from_utf8_lossy(&body);
        let detail = service_error(&text).unwrap_or_else(|| text.chars().take(160).collect());
        return Err(format!("HTTP {status} van {} — {}", host_of(url), detail.trim()));
    }
    Ok(Fetched { content_type, body })
}

pub fn get(url: &str) -> Result<Fetched, String> {
    finish(url, default_agent().get(url).header("Accept-Language", "nl,en").call())
}

pub fn get_text(url: &str) -> Result<String, String> {
    let f = get(url)?;
    Ok(decode_text(f.body))
}

/// UTF-8, or else Latin-1 (some services answer in ISO-8859-1, e.g. the
/// Spanish cadastre): every byte one character, and an XML declaration that
/// names the old encoding says UTF-8 from then on.
pub fn decode_text(body: Vec<u8>) -> String {
    match String::from_utf8(body) {
        Ok(s) => s,
        Err(e) => {
            let text: String = e.into_bytes().iter().map(|b| *b as char).collect();
            match text.find("?>").filter(|_| text.starts_with("<?xml")) {
                Some(end) => {
                    let decl = &text[..end];
                    let fixed = ["ISO-8859-1", "iso-8859-1", "ISO-8859-15", "windows-1252", "Windows-1252"].iter().fold(decl.to_string(), |d, enc| d.replace(enc, "UTF-8"));
                    format!("{fixed}{}", &text[end..])
                }
                None => text,
            }
        }
    }
}

pub fn get_json(url: &str) -> Result<serde_json::Value, String> {
    let text = get_text(url)?;
    serde_json::from_str(&text).map_err(|e| {
        service_error(&text).unwrap_or_else(|| format!("{}: ongeldige JSON ({e})", host_of(url)))
    })
}

/// Bytes `start..=end` of a remote file (HTTP range request), for cloud
/// optimised formats such as COPC.
pub fn get_range(url: &str, start: u64, end: u64) -> Result<Vec<u8>, String> {
    let range = format!("bytes={start}-{end}");
    let mut last = String::new();
    // Object storage occasionally drops a connection: try a few times.
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(500 * attempt));
        }
        match finish(url, default_agent().get(url).header("Range", &range).call()) {
            Ok(f) => return Ok(f.body),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Size of a remote file from a HEAD request.
pub fn content_length(url: &str) -> Result<u64, String> {
    let resp = default_agent().head(url).call().map_err(|e| format!("{}: {e}", host_of(url)))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("HTTP {status} van {}", host_of(url)));
    }
    resp.headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("{}: geen bestandsgrootte", host_of(url)))
}

/// POST `application/x-www-form-urlencoded` with an explicit timeout.
pub fn post_form(url: &str, form: &[(&str, &str)], timeout_s: u64) -> Result<Fetched, String> {
    finish(url, agent(timeout_s).post(url).send_form(form.iter().copied()))
}

/// POST a JSON body (search APIs such as the BRO's) with an explicit timeout.
pub fn post_json(url: &str, body: &serde_json::Value, timeout_s: u64) -> Result<Fetched, String> {
    finish(url, agent(timeout_s).post(url).header("Content-Type", "application/json").header("Accept-Language", "nl,en").send(body.to_string()))
}

/// Fetch an image; an XML/HTML answer is turned into the server's own error.
pub fn get_image(url: &str) -> Result<Vec<u8>, String> {
    let f = get(url)?;
    if !f.content_type.starts_with("image/") {
        let text = String::from_utf8_lossy(&f.body);
        return Err(service_error(&text).unwrap_or_else(|| {
            format!("server gaf geen afbeelding terug ({})", if f.content_type.is_empty() { "onbekend type" } else { &f.content_type })
        }));
    }
    Ok(f.body)
}

/// The message of an OGC ServiceException / ows:ExceptionText.
pub fn service_error(text: &str) -> Option<String> {
    // OGC exceptions, and the BRO's `brocom:rejectionReason`.
    for tag in ["<ServiceException>", "<ServiceException ", ":ServiceException>", ":ServiceException ", ":ExceptionText>", "<ExceptionText>", ":rejectionReason>"] {
        let Some(open) = text.find(tag) else { continue };
        let Some(start) = text[open..].find('>').map(|i| open + i + 1) else { continue };
        let Some(end) = text[start..].find("</").map(|i| start + i) else { continue };
        let msg = text[start..end].replace("<![CDATA[", "").replace("]]>", "");
        let msg = msg.trim();
        if !msg.is_empty() {
            return Some(msg.to_string());
        }
    }
    None
}

/// Percent-encode a query component.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Add/replace query parameters on a service URL.
pub fn with_params(base: &str, params: &[(&str, String)]) -> String {
    let (path, query) = base.split_once('?').unwrap_or((base, ""));
    let mut kept: Vec<String> = query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .filter(|kv| {
            let k = kv.split('=').next().unwrap_or("");
            !params.iter().any(|(p, _)| p.eq_ignore_ascii_case(k))
        })
        .map(str::to_string)
        .collect();
    kept.extend(params.iter().map(|(k, v)| format!("{k}={}", encode(v))));
    format!("{path}?{}", kept.join("&"))
}

/// Remove the OGC request parameters a user may have pasted with a URL.
pub fn clean_service_url(url: &str) -> String {
    let url = url.trim();
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    let kept: Vec<&str> = query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .filter(|kv| {
            let k = kv.split('=').next().unwrap_or("").to_ascii_lowercase();
            !matches!(
                k.as_str(),
                "service" | "request" | "version" | "layers" | "typename" | "typenames" | "bbox" | "srs" | "crs"
                    | "format" | "width" | "height" | "styles" | "outputformat"
            )
        })
        .collect();
    if kept.is_empty() { path.to_string() } else { format!("{path}?{}", kept.join("&")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latin1_text() {
        let t = decode_text(b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>Espa\xf1a</a>".to_vec());
        assert_eq!(t, "<?xml version=\"1.0\" encoding=\"UTF-8\"?><a>España</a>");
    }

    #[test]
    fn params_replace_existing_case_insensitively() {
        let u = with_params("https://x/wms?map=a&SERVICE=WMS", &[("service", "WFS".into()), ("bbox", "1,2".into())]);
        assert_eq!(u, "https://x/wms?map=a&service=WFS&bbox=1%2C2");
    }

    #[test]
    fn exception_text_is_extracted() {
        let xml = "<ows:ExceptionReport><ows:Exception><ows:ExceptionText>Bad layer</ows:ExceptionText></ows:Exception></ows:ExceptionReport>";
        assert_eq!(service_error(xml).as_deref(), Some("Bad layer"));
    }
}
