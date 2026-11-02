//! Command-line and interactive entry points for the infrastructure library.
use super::{host, Link, Placement};
use crate::{
    bridge,
    civiel::api::{self, Args},
};
use ocs_plugin_api::host::{CommandStep, HostApi, InteractiveCommand, PreviewWire};
use std::collections::BTreeMap;

fn unique() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "infra-{:x}-{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}
pub fn parse(text: &str) -> Result<Placement, String> {
    let a = Args::parse(text);
    let item = super::find(
        a.str("ITEM")
            .or_else(|| a.words.first().map(String::as_str))
            .ok_or("ITEM ontbreekt")?,
    )?;
    let mut p = Placement {
        item: item.id.clone(),
        instance: a.str("INSTANCE").map(str::to_string).unwrap_or_else(unique),
        ..Default::default()
    };
    if let Some(pos) = a.str("AT") {
        let nums: Vec<f64> = pos
            .split(',')
            .map(|n| n.parse().map_err(|_| "AT=x,y[,z]".to_string()))
            .collect::<Result<_, _>>()?;
        if !(2..=3).contains(&nums.len()) {
            return Err("AT=x,y[,z]".into());
        }
        p.at = [nums[0], nums[1], *nums.get(2).unwrap_or(&0.)];
        p.elevation_known = nums.len() == 3;
    }
    p.rotation = a.f64("ROTATION")?.unwrap_or(0.).to_radians();
    p.scale = a.f64("SCALE")?.unwrap_or(500.);
    p.status = a.str("STATUS").unwrap_or("N").to_uppercase();
    p.discipline = a.str("DISCIPLINE").unwrap_or("XX").to_uppercase();
    for key in a.keys() {
        if let Some(param) = item.parameters.iter().find(|x| x.key.to_uppercase() == key) {
            p.values
                .insert(param.key.clone(), a.f64(key)?.ok_or("parameter")?);
        } else if ![
            "ITEM",
            "INSTANCE",
            "AT",
            "ROTATION",
            "SCALE",
            "STATUS",
            "DISCIPLINE",
            "PATH",
            "CONNECT",
            "PORT",
            "OWN",
            "ID",
            "CONFIRM",
        ]
        .contains(&key)
        {
            return Err(format!("onbekend argument {key}"));
        }
    }
    if let Some(path) = a.points("PATH")? {
        let (s, c) = p.rotation.sin_cos();
        p.path = path
            .into_iter()
            .map(|v| {
                let (x, y) = (v[0] - p.at[0], v[1] - p.at[1]);
                [c * x + s * y, -s * x + c * y]
            })
            .collect();
    }
    if let Some(instance) = a.str("CONNECT") {
        p.link = Some(Link {
            instance: instance.into(),
            port: a.str("PORT").unwrap_or("out").into(),
            own_port: a.str("OWN").unwrap_or("in").into(),
        });
    }
    item.validate(&p)?;
    Ok(p)
}
fn worker(f: impl FnOnce() -> Result<serde_json::Value, String> + Send + 'static) {
    static MUTATIONS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let Some(target) = bridge::target() else {
        bridge::error("geen tekening actief");
        return;
    };
    let site = bridge::site();
    let reply = bridge::take_reply_id();
    std::thread::spawn(move || {
        let _serial = MUTATIONS.lock().unwrap_or_else(|e| e.into_inner());
        bridge::with_command_context(target, site, reply, || match f() {
            Ok(v) => {
                api::note("infra", v);
                bridge::info("Infra-opdracht voltooid");
            }
            Err(e) => bridge::error(&e),
        });
    });
}
pub fn dispatch(host_api: &mut dyn HostApi, cmd: &str) {
    let (verb, text) = cmd.split_once(char::is_whitespace).unwrap_or((cmd, ""));
    let a = Args::parse(text);
    match verb.to_ascii_uppercase().as_str() {
        "GIS_INFRA_CATALOG" | "GIS_INFRA_CATALOGUS" => {
            match super::search(
                a.str("QUERY").unwrap_or(""),
                a.str("CATEGORY").unwrap_or(""),
            ) {
                Ok(items) => {
                    api::note("items", serde_json::to_value(items).unwrap_or_default());
                    bridge::reply(true, "Infra-catalogus");
                    host_api.push_info("[Open Infra] Infra-catalogus beschikbaar; GIS_INFRA_LIBRARY opent het venster.");
                }
                Err(e) => {
                    host_api.push_error(&e);
                    bridge::reply(false, &e);
                }
            }
        }
        "GIS_INFRA_INSPECT" => worker(move || {
            let doc = bridge::snapshot()?;
            let objects = host::scene(&doc)?;
            let filtered: Vec<_> = objects
                .into_iter()
                .filter(|p| a.str("INSTANCE").is_none_or(|id| p.instance == id))
                .map(|p| host::result(super::find(&p.item)?, &p, &[]))
                .collect::<Result<_, String>>()?;
            Ok(serde_json::json!({"objects":filtered}))
        }),
        "GIS_INFRA_UPDATE" => {
            worker(move || {
                let instance = a.str("INSTANCE").ok_or("INSTANCE ontbreekt")?;
                let doc = bridge::snapshot()?;
                let scene = host::scene(&doc)?;
                let p = scene
                    .iter()
                    .find(|p| p.instance == instance)
                    .ok_or("instantie niet gevonden")?;
                let item = super::find(&p.item)?;
                let mut values = BTreeMap::new();
                for key in a.keys() {
                    if key == "INSTANCE" || key == "PATH" {
                        continue;
                    }
                    let param = item
                        .parameters
                        .iter()
                        .find(|p| p.key.to_uppercase() == key)
                        .ok_or_else(|| format!("onbekende parameter {key}"))?;
                    values.insert(param.key.clone(), a.f64(key)?.ok_or("waarde")?);
                }
                let path = a.points("PATH")?;
                if values.is_empty() && path.is_none() {
                    return Err("geen gewijzigde parameters of pad".into());
                }
                host::update(instance, values, path)
            });
        }
        _ => match parse(text) {
            Ok(p) => {
                let item = super::find(&p.item).expect("validated item");
                // Conflicts must be acknowledged through an explicit command argument.
                if !item.conflicts.is_empty() && a.str("CONFIRM") != Some("true") {
                    let e=format!("{}: bronconflict; controleer catalogus en geef CONFIRM=true met expliciete parameters",item.name());
                    host_api.push_error(&e);
                    bridge::reply(false, &e);
                    return;
                }
                if !item.conflicts.is_empty()
                    && item.parameters.iter().any(|v| v.key == "slope")
                    && a.str("SLOPE").is_none()
                {
                    let e="bronconflict: geef SLOPE expliciet op; de cataloguswaarde nul is geen ontwerpafschot";
                    host_api.push_error(e);
                    bridge::reply(false, e);
                    return;
                }
                if a.str("AT").is_some() || a.str("PATH").is_some() || p.link.is_some() {
                    worker(move || host::place(p));
                } else {
                    host_api.start_interactive(Box::new(Insert::new(p)));
                }
            }
            Err(e) => {
                host_api.push_error(&e);
                bridge::reply(false, &e);
            }
        },
    }
}

pub struct Insert {
    p: Placement,
    points: Vec<[f64; 2]>,
}
impl Insert {
    pub fn new(p: Placement) -> Self {
        Self { p, points: vec![] }
    }
    fn project(&self, pt: [f64; 3]) -> [f64; 2] {
        let u = bridge::target().map(|t| t.unit_factor).unwrap_or(1.);
        bridge::site()
            .map(|s| s.from_drawing([pt[0], pt[1]], u))
            .unwrap_or([pt[0] / u, pt[1] / u])
    }
    fn finish(&self) -> CommandStep {
        let mut p = self.p.clone();
        let item = match super::find(&p.item) {
            Ok(i) => i,
            Err(_) => return CommandStep::Cancel,
        };
        if self.points.is_empty() {
            return CommandStep::Cancel;
        }
        if matches!(item.kind.as_str(), "line" | "area") {
            if self.points.len() < if item.kind == "area" { 3 } else { 2 } {
                bridge::error("onvoldoende punten");
                return CommandStep::Cancel;
            }
            p.at[0] = self.points[0][0];
            p.at[1] = self.points[0][1];
            p.path = self
                .points
                .iter()
                .map(|v| [v[0] - p.at[0], v[1] - p.at[1]])
                .collect();
        } else {
            p.at[0] = self.points[0][0];
            p.at[1] = self.points[0][1];
            if self.points.len() > 1 {
                p.rotation = (self.points[1][1] - p.at[1]).atan2(self.points[1][0] - p.at[0]);
            }
        }
        worker(move || host::place(p));
        CommandStep::Done
    }
}
impl InteractiveCommand for Insert {
    fn prompt(&self) -> String {
        let kind = super::find(&self.p.item)
            .map(|i| i.kind.as_str())
            .unwrap_or("point");
        if matches!(kind, "line" | "area") {
            "Infra: volgende hoekpunt; Enter = plaatsen, Esc = annuleren".into()
        } else if self.points.is_empty() {
            "Infra: invoegpunt".into()
        } else {
            "Infra: richting; Enter = oost".into()
        }
    }
    fn on_point(&mut self, pt: [f64; 3]) -> CommandStep {
        let p = self.project(pt);
        self.points.push(p);
        let kind = super::find(&self.p.item)
            .map(|i| i.kind.as_str())
            .unwrap_or("point");
        if !matches!(kind, "line" | "area") && self.points.len() == 2 {
            self.finish()
        } else {
            CommandStep::NeedPoint
        }
    }
    fn on_enter(&mut self) -> CommandStep {
        self.finish()
    }
    fn on_cursor_move(&mut self, pt: [f64; 3]) -> Vec<PreviewWire> {
        let Some(a) = self.points.first() else {
            return vec![];
        };
        let u = bridge::target().map(|t| t.unit_factor).unwrap_or(1.);
        let site = bridge::site();
        let mut pts: Vec<_> = self
            .points
            .iter()
            .map(|p| {
                let v = site
                    .as_ref()
                    .map(|s| s.to_drawing(*p, u))
                    .unwrap_or([p[0] * u, p[1] * u]);
                [v[0], v[1], 0.]
            })
            .collect();
        let _ = a;
        pts.push(pt);
        vec![PreviewWire::with_color(pts, [0.3, 0.8, 1., 1.])]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_checks_unknown_and_nonfinite_and_converts_absolute_path() {
        assert!(parse("ITEM=zitbank WIDTH=NaN").is_err());
        assert!(parse("ITEM=zitbank TYPO=2").is_err());
        let p =
            parse("ITEM=opsluitband-150x250 AT=100,200 ROTATION=90 PATH=100,200;100,205").unwrap();
        assert!((p.path[1][0] - 5.).abs() < 1e-8);
        assert!(p.path[1][1].abs() < 1e-8);
    }
}
