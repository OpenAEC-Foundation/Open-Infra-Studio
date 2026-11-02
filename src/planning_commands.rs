//! Planning commands. Credentials are read from the local process environment.
use crate::{
    bridge,
    civiel::api,
    gis::{crs, planning},
};
pub fn dispatch(cmd: &str) {
    let (verb, rest) = cmd.split_once(char::is_whitespace).unwrap_or((cmd, ""));
    let verb = verb.to_ascii_uppercase();
    let args = api::Args::parse(rest);
    let Some(target) = bridge::target() else {
        bridge::error("Geen tekening actief");
        return;
    };
    let site = bridge::site();
    let reply = bridge::take_reply_id();
    let path = rest.trim().trim_matches('"').to_string();
    std::thread::spawn(move || {
        bridge::with_command_context(target, site.clone(), reply, || {
            let result = (|| -> Result<String, String> {
                if verb == "GIS_PLAN_GML" {
                    let site = site.as_ref().ok_or("Stel eerst GIS_SITE in")?;
                    if path.is_empty() {
                        return Err("GIS_PLAN_GML <volledig pad naar GML>".into());
                    }
                    return crate::plannen::import_path(site, std::path::Path::new(&path), &|_| {});
                }
                let client = planning::Client::new(
                    std::env::var("OPENINFRA_RP_API_KEY").unwrap_or_default(),
                )?;
                if verb == "GIS_PLAN_SEARCH" {
                    let point = match (args.f64("X")?, args.f64("Y")?) {
                        (Some(x), Some(y)) => [x, y],
                        (None, None) => {
                            let site = site
                                .as_ref()
                                .ok_or("Geef X= en Y= in RD, of stel GIS_SITE in")?;
                            site.register_crs();
                            crs::transform(&site.crs, "EPSG:28992", [site.origin_e, site.origin_n])?
                        }
                        _ => return Err("Geef zowel X= als Y= in RD".into()),
                    };
                    let plans = client.search(point)?;
                    let count = plans.len();
                    api::note(
                        "plans",
                        serde_json::to_value(plans).map_err(|e| e.to_string())?,
                    );
                    Ok(format!("{count} IMRO-plannen gevonden"))
                } else {
                    let site = site.as_ref().ok_or("Stel eerst GIS_SITE in")?;
                    let id = args.str("PLAN").ok_or("GIS_PLAN_IMPORT PLAN=<plan-ID>")?;
                    let data = client.download(id)?;
                    crate::plannen::place(
                        site,
                        &data,
                        &format!("{}/plannen/{id}", planning::BASE),
                        &|_| {},
                    )
                }
            })();
            match result {
                Ok(message) => bridge::info(&message),
                Err(e) => bridge::error(&e),
            }
        })
    });
}
