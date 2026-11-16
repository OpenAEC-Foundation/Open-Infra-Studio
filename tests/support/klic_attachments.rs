//! Kleine compilatie van de productiecode. Alleen taalkeuze en XY zijn hier
//! vervangen; uitpakken, padcontrole en openen komen uit het echte bestand.
#![allow(dead_code)]

mod lang {
    pub fn tr<'a>(nl: &'a str, _en: &'a str) -> &'a str { nl }
}
mod crs {
    pub type XY = [f64; 2];
}
#[path = "../../src/xmldoc.rs"]
mod xmldoc;
#[path = "../../src/gis/klic_extra.rs"]
mod klic_extra;

#[cfg(windows)]
#[path = "klic_windows.rs"]
mod windows;
