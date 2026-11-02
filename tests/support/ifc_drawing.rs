//! Projectlocatie uit de opgeslagen DWG, gedeeld door de IFC-exporttests.
use ocs_plugin_api::host::codec::{
    objects::{ObjectType, XRecordValue},
    CadDocument,
};
use open_infra::gis::site::Site;

pub fn site_of(doc: &CadDocument) -> Option<Site> {
    doc.objects.values().find_map(|o| match o {
        ObjectType::XRecord(x)
            if x.name.eq_ignore_ascii_case("SITE")
                || x.entries.iter().any(
                    |e| matches!(&e.value, XRecordValue::String(s) if s.contains("crs_proj4")),
                ) =>
        {
            x.entries.iter().find_map(|e| match &e.value {
                XRecordValue::String(s) if s.trim_start().starts_with('{') => {
                    serde_json::from_str(s).ok()
                }
                _ => None,
            })
        }
        _ => None,
    })
}
