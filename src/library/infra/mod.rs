//! Parametric infrastructure catalogue. All geometry and connector coordinates are metres.
pub mod commands;
pub mod connect;
pub mod geometry;
pub mod host;

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    sync::OnceLock,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Parameter {
    pub key: String,
    pub label: String,
    pub default: f64,
    pub unit: String,
    pub min: f64,
    pub max: f64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Source {
    pub document: String,
    pub revision: String,
    pub location: String,
    pub url: String,
    pub license: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MaterialLayer {
    pub material: String,
    pub thickness: f64,
    pub object_id: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Item {
    pub id: String,
    pub name_nl: String,
    pub name_en: String,
    pub category: String,
    pub object_id: u32,
    pub nlcs_version: String,
    pub element: String,
    pub kind: String,
    pub shape: String,
    pub system: String,
    pub material: String,
    pub revision: u32,
    pub keywords: Vec<String>,
    pub parameters: Vec<Parameter>,
    pub port_count: usize,
    pub sources: Vec<Source>,
    pub value_status: String,
    pub conflicts: Vec<String>,
    pub quantity_unit: String,
    pub layers: Vec<MaterialLayer>,
    pub components: Vec<MaterialLayer>,
}
impl Item {
    pub fn name(&self) -> &str {
        crate::lang::tr(&self.name_nl, &self.name_en)
    }
    pub fn value(&self, p: &Placement, key: &str) -> f64 {
        p.values
            .get(key)
            .copied()
            .or_else(|| {
                self.parameters
                    .iter()
                    .find(|x| x.key == key)
                    .map(|x| x.default)
            })
            .unwrap_or(0.0)
    }
    pub fn validate(&self, p: &Placement) -> Result<(), String> {
        if !p.instance.is_empty()
            && (p.instance.len() > 128
                || !p
                    .instance
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        {
            return Err("instantie-ID: maximaal 128 letters, cijfers, - of _".into());
        }
        if !p.path.is_empty() && !matches!(self.kind.as_str(), "line" | "area") {
            return Err("PATH is alleen toegestaan voor lijnen en vlakken".into());
        }
        if !p
            .at
            .iter()
            .chain(p.path.iter().flatten())
            .chain([&p.rotation, &p.scale])
            .all(|v| v.is_finite())
            || p.scale <= 0.0
            || p.scale > 100000.0
        {
            return Err("coördinaten, rotatie en schaal moeten eindig zijn; schaal > 0".into());
        }
        for (key, value) in &p.values {
            let spec = self
                .parameters
                .iter()
                .find(|x| &x.key == key)
                .ok_or_else(|| format!("onbekende parameter {key}"))?;
            if !value.is_finite() || *value < spec.min || *value > spec.max {
                return Err(format!(
                    "{key}: {} t/m {} {}",
                    spec.min, spec.max, spec.unit
                ));
            }
        }
        if crate::nlcs::Status::from_code(&p.status).is_none() {
            return Err("status: B, N, T of V".into());
        }
        if !crate::nlcs::disciplines()
            .iter()
            .any(|(c, _)| *c == p.discipline)
        {
            return Err("onbekende discipline".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Link {
    pub instance: String,
    pub port: String,
    pub own_port: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Placement {
    pub elevation_known: bool,
    pub at: [f64; 3],
    pub rotation: f64,
    pub scale: f64,
    pub values: BTreeMap<String, f64>,
    /// Local coordinates in metres, relative to at, rotated by rotation.
    pub path: Vec<[f64; 2]>,
    pub status: String,
    pub discipline: String,
    pub link: Option<Link>,
    pub instance: String,
    pub item: String,
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            elevation_known: false,
            at: [0.; 3],
            rotation: 0.,
            scale: 500.,
            values: BTreeMap::new(),
            path: vec![],
            status: "N".into(),
            discipline: "XX".into(),
            link: None,
            instance: String::new(),
            item: String::new(),
        }
    }
}

pub fn catalog() -> Result<&'static [Item], String> {
    static C: OnceLock<Result<Vec<Item>, String>> = OnceLock::new();
    C.get_or_init(|| {
        let items: Vec<Item> =
            serde_json::from_str(include_str!("../../../assets/library/infra.json"))
                .map_err(|e| e.to_string())?;
        let mut ids = HashSet::new();
        for i in &items {
            if !["point", "line", "area", "profile", "annotation"].contains(&i.kind.as_str())
                || ![
                    "box", "surface", "curb", "pipe", "valve", "check", "vent", "bend", "tee",
                    "reducer", "chamber", "hydrant", "lamp", "charger", "tree", "bench", "rack",
                    "sign", "gate", "posts", "log", "fence", "line", "label", "section",
                ]
                .contains(&i.shape.as_str())
                || !["m", "m2", "st"].contains(&i.quantity_unit.as_str())
            {
                return Err(format!("{}: onbekende soort, vorm of meetregel", i.id));
            }
            if !ids.insert(i.id.clone()) {
                return Err(format!("dubbel ID {}", i.id));
            }
            let obj = crate::nlcs::object(i.object_id)
                .ok_or_else(|| format!("{}: onbekend NLCS-object", i.id))?;
            if i.nlcs_version != "5.0.2"
                || !crate::nlcs::element_allowed(obj, &i.element)
                || i.sources.is_empty()
            {
                return Err(format!("{}: ongeldige mapping of bron", i.id));
            }
            let mut keys = HashSet::new();
            for component in i.components.iter().chain(&i.layers) {
                if crate::nlcs::object(component.object_id).is_none()
                    || !component.thickness.is_finite()
                    || component.thickness <= 0.0
                {
                    return Err(format!("{}: ongeldige materiaalcomponent", i.id));
                }
            }
            for p in &i.parameters {
                if !keys.insert(&p.key)
                    || ![p.default, p.min, p.max].iter().all(|x| x.is_finite())
                    || p.min > p.default
                    || p.default > p.max
                {
                    return Err(format!("{}: ongeldige parameter {}", i.id, p.key));
                }
            }
        }
        Ok(items)
    })
    .as_ref()
    .map(Vec::as_slice)
    .map_err(Clone::clone)
}
pub fn find(id: &str) -> Result<&'static Item, String> {
    catalog()?
        .iter()
        .find(|i| i.id.eq_ignore_ascii_case(id))
        .ok_or_else(|| format!("onbekend infra-object {id}"))
}
pub fn search(query: &str, category: &str) -> Result<Vec<&'static Item>, String> {
    let q = query.to_lowercase();
    Ok(catalog()?
        .iter()
        .filter(|i| category.is_empty() || i.category == category)
        .filter(|i| {
            let s = format!(
                "{} {} {} {} {}",
                i.id,
                i.name_nl,
                i.name_en,
                i.material,
                i.keywords.join(" ")
            )
            .to_lowercase();
            q.split_whitespace().all(|w| s.contains(w))
        })
        .collect())
}
