//! Vehicle library: design vehicles from published standards (AASHTO, FGSV,
//! Austroads, EU maximum dimensions, ...) and common vehicles for drawings,
//! drawn as DWG blocks in plan and side view.
//!
//! The data is `assets/library/vehicles.json`: per vehicle its units (a
//! tractor and a semitrailer, a bus and its rear section, ...) with length,
//! width, axle positions and the coupling points. [`draw`] turns that into
//! line work; [`place`] makes the blocks and inserts them.

pub mod draw;
pub mod parking;
pub mod place;
pub mod swept;

use std::sync::OnceLock;

use serde::Deserialize;

use crate::lang::{self, Lang};

#[derive(Clone, Debug, Deserialize)]
pub struct Axle {
    /// From the front of the unit (m).
    pub x: f64,
    /// Track: distance between the wheel centres, or between the centres of
    /// the twin pairs (m).
    pub track: f64,
    #[serde(default)]
    pub dual: bool,
    #[serde(default)]
    pub steer: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Unit {
    pub kind: String,
    pub length: f64,
    pub width: f64,
    #[serde(default)]
    pub front_overhang: Option<f64>,
    #[serde(default)]
    pub axles: Vec<Axle>,
    /// Coupling on this unit for the next one (fifth wheel, hitch), from
    /// the front of this unit.
    #[serde(default)]
    pub hitch_x: Option<f64>,
    /// Where this unit hangs on the previous one (kingpin, drawbar eye), from
    /// the front of this unit; negative when the drawbar sticks out in front.
    #[serde(default)]
    pub kingpin_x: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Vehicle {
    pub id: String,
    pub name_en: String,
    pub name_nl: String,
    pub category: String,
    #[serde(default)]
    pub standard: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub height: Option<f64>,
    #[serde(default)]
    pub turning_radius_outer: Option<f64>,
    pub units: Vec<Unit>,
    #[serde(default)]
    pub estimated: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

impl Vehicle {
    pub fn name(&self) -> &str {
        match lang::current() {
            Lang::Nl => &self.name_nl,
            Lang::En => &self.name_en,
        }
    }

    /// Overall length with the units coupled, front of the first unit to
    /// the rear of the last (m).
    pub fn length(&self) -> f64 {
        let fronts = draw::unit_fronts(self);
        let rear = self.units.iter().zip(&fronts).map(|(u, d)| d + u.length).fold(0.0, f64::max);
        let front = fronts.iter().copied().fold(0.0, f64::min);
        rear - front
    }

    pub fn width(&self) -> f64 {
        self.units.iter().map(|u| u.width).fold(0.0, f64::max)
    }

    /// A short line with the main dimensions, for lists and object data.
    pub fn dimensions(&self) -> String {
        let mut s = format!("{:.2} × {:.2} m", self.length(), self.width());
        if let Some(h) = self.height {
            s.push_str(&format!(" × {h:.2} m"));
        }
        if let Some(r) = self.turning_radius_outer {
            s.push_str(&format!(", R {r:.1} m"));
        }
        s
    }

    /// Block name in the drawing: `OI_<ID>_TOP` or `OI_<ID>_SIDE`.
    pub fn block_name(&self, view: draw::View) -> String {
        let id: String = self.id.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
        format!("OI_{id}_{}", view.suffix())
    }
}

/// Categories in display order, with their names.
pub const CATEGORIES: &[(&str, &str, &str)] = &[
    ("car", "Personenauto's", "Cars"),
    ("van", "Bestelauto's", "Vans"),
    ("pickup", "Pick-ups", "Pick-ups"),
    ("truck", "Vrachtauto's", "Trucks"),
    ("tractor_semitrailer", "Trekker met oplegger", "Tractor-semitrailers"),
    ("truck_trailer", "Vrachtauto met aanhanger", "Truck-trailers"),
    ("bus", "Bussen", "Buses"),
    ("articulated_bus", "Gelede bussen", "Articulated buses"),
    ("coach", "Touringcars", "Coaches"),
    ("emergency", "Hulpdiensten", "Emergency vehicles"),
    ("municipal", "Gemeentelijke voertuigen", "Municipal vehicles"),
    ("construction", "Bouwmachines", "Construction machinery"),
    ("agricultural", "Landbouw", "Agricultural"),
    ("two_wheeler", "Fietsen en tweewielers", "Bicycles and two-wheelers"),
    ("rail", "Rail en tram", "Rail and tram"),
    ("special", "Overig", "Other"),
];

pub fn category_name(code: &str) -> String {
    CATEGORIES
        .iter()
        .find(|(c, ..)| *c == code)
        .map(|(_, nl, en)| lang::tr(nl, en).to_string())
        .unwrap_or_else(|| code.to_string())
}

/// All vehicles, in category order.
pub fn vehicles() -> &'static [Vehicle] {
    static V: OnceLock<Vec<Vehicle>> = OnceLock::new();
    V.get_or_init(|| {
        let mut v: Vec<Vehicle> = serde_json::from_str(include_str!("../../assets/library/vehicles.json")).unwrap_or_default();
        let rank = |c: &str| CATEGORIES.iter().position(|(k, ..)| *k == c).unwrap_or(CATEGORIES.len());
        v.retain(|x| !x.units.is_empty() && x.units.iter().all(|u| u.length > 0.0 && u.width > 0.0));
        v.sort_by(|a, b| rank(&a.category).cmp(&rank(&b.category)).then(a.length().total_cmp(&b.length())));
        v
    })
}

pub fn find(id: &str) -> Option<&'static Vehicle> {
    let id = id.trim();
    vehicles().iter().find(|v| v.id.eq_ignore_ascii_case(id) || v.block_name(draw::View::Top).eq_ignore_ascii_case(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_loads_and_is_consistent() {
        let all = vehicles();
        assert!(!all.is_empty(), "vehicles.json did not parse");
        let mut ids = std::collections::HashSet::new();
        for v in all {
            assert!(ids.insert(v.id.clone()), "duplicate id {}", v.id);
            for u in &v.units {
                for a in &u.axles {
                    assert!(a.x >= -0.01 && a.x <= u.length + 0.01, "{}: axle at {} outside a unit of {}", v.id, a.x, u.length);
                }
            }
            assert!(v.length() > 0.3 && v.length() < 60.0, "{}: length {}", v.id, v.length());
        }
    }
}
