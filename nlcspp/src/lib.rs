//! NLCS++ Netbeheer: the sector standard of the Dutch network operators for
//! design and as-built drawings of cables and pipes (DWG + XML).
//!
//! - [`model`]: the V12.1 information model (56 object types), the general
//!   code lists and those of each operator (Alliander, Enexis, Stedin), from
//!   the official XSDs of mijnaansluiting.nl;
//! - [`xml`]: read deliveries (V11, V12, V12.1, every operator) and write
//!   V12.1;
//! - [`layer`]: the layer-name grammar both ways, on the layers of the
//!   operators' DWG templates;
//! - [`check`]: findings at the levels Fout, Waarschuwing and Informerend.
//!
//! Host-independent: no CAD types, so other programs can use it too.

pub mod check;
pub mod layer;
pub mod model;
pub mod xml;

pub use check::{Finding, Level};
pub use xml::{Delivery, Geom, Object};
