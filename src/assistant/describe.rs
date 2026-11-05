//! A command line in plain words, for the activity log and the proposals:
//! "Create an alignment: Rondweg Zuid" for `CIV_AS PI=… NAAM="Rondweg Zuid"`.

use crate::civiel::api::Args;

/// Verb → (Dutch, English).
const TITLES: &[(&str, &str, &str)] = &[
    ("CIV_TIN", "Terrein (TIN) maken", "Create a terrain (TIN)"),
    ("CIV_HOOGTELIJNEN", "Hoogtelijnen tekenen", "Draw contours"),
    ("CIV_ANALYSE", "Hoogteanalyse tekenen", "Draw an elevation analysis"),
    ("CIV_VOLUME", "Grondverzet berekenen", "Compute earthworks"),
    ("CIV_VOLUME_TIN", "Grondverzet TIN-TIN berekenen", "Compute TIN-to-TIN earthworks"),
    ("CIV_GRONDBALANS", "Grondbalans tekenen", "Draw a mass haul diagram"),
    ("CIV_AS", "As maken", "Create an alignment"),
    ("CIV_AS_OFFSET", "As verschuiven", "Offset an alignment"),
    ("CIV_STATION", "Station en offset bepalen", "Find station and offset"),
    ("CIV_LENGTEPROFIEL", "Lengteprofiel tekenen", "Draw a length profile"),
    ("CIV_ONTWERPPROFIEL", "Ontwerpprofiel maken", "Create a design profile"),
    ("CIV_DWARSPROFIELEN", "Dwarsprofielen tekenen", "Draw cross sections"),
    ("CIV_CORRIDOR", "Corridor maken", "Create a corridor"),
    ("CIV_KRUISPUNT", "Kruispunt maken", "Create a junction"),
    ("CIV_ROTONDE", "Rotonde maken", "Create a roundabout"),
    ("CIV_PROFIELTYPE", "Profieltype tonen", "Show an assembly"),
    ("CIV_KANTLIJN", "Kantlijn maken", "Create a feature line"),
    ("CIV_KANTLIJN_PEIL", "Kantlijn: peil van een hoekpunt", "Feature line: set a corner level"),
    ("CIV_KANTLIJN_VERHOOG", "Kantlijn verhogen of verlagen", "Raise or lower a feature line"),
    ("CIV_KANTLIJN_PUNT", "Kantlijn: hoogtepunt invoegen", "Feature line: insert a height point"),
    ("CIV_KANTLIJN_HELLING", "Kantlijn: helling tussen hoekpunten", "Feature line: grade between corners"),
    ("CIV_PLATEAU", "Plateau maken", "Create a plateau"),
    ("CIV_TALUD", "Talud maken", "Create a slope"),
    ("CIV_PUNT", "Meetpunten plaatsen", "Place survey points"),
    ("CIV_PUNTEN_IMPORT", "Meetpunten importeren", "Import survey points"),
    ("CIV_PUNTEN_EXPORT", "Meetpunten exporteren", "Export survey points"),
    ("CIV_LANDXML", "LandXML importeren", "Import LandXML"),
    ("CIV_LANDXML_EXPORT", "LandXML exporteren", "Export LandXML"),
    ("CIV_IFC", "IFC 4.3 exporteren", "Export IFC 4.3"),
    ("CIV_BAG3D", "3D BAG-gebouwen", "3D BAG buildings"),
    ("CIV_BGT3D", "BGT in 3D", "BGT in 3D"),
    ("CIV_3DMODEL", "3D-model maken", "Create the 3D model"),
    ("CIV_FOTOTERREIN", "Terrein met luchtfoto", "Terrain with aerial photo"),
    ("CIV_WATERPEIL", "Waterpeil tonen", "Show a water level"),
    ("CIV_DOORZICHT", "Maaiveld doorzichtig maken", "Make the ground see-through"),
    ("CIV_KLIC3D", "KLIC in 3D", "KLIC in 3D"),
    ("CIV_CITYGML", "CityGML of CityJSON importeren", "Import CityGML or CityJSON"),
    ("CIV_SONDERINGEN3D", "Sonderingen en boringen in 3D", "CPTs and boreholes in 3D"),
    ("CIV_RIOOL", "Riool ontwerpen", "Design a sewer"),
    ("CIV_RIOOL_BIJWERKEN", "Riool bijwerken", "Update a sewer"),
    ("CIV_RIOOL_PROFIEL", "Rioolprofiel tekenen", "Draw a sewer profile"),
    ("CIV_RIOOL_TOETS", "Riool toetsen", "Check a sewer"),
    ("CIV_RIOOL_EXPORT", "Riool exporteren", "Export a sewer"),
    ("CIV_BLADEN", "Bladen maken", "Create sheets"),
    ("CIV_BLADEN_PDF", "Bladen naar PDF", "Sheets to PDF"),
    ("CIV_LABELS", "Labels plaatsen", "Place labels"),
    ("CIV_LABELS_BIJWERKEN", "Labels bijwerken", "Update labels"),
    ("CIV_BIJWERKEN", "Profielen bijwerken", "Update profiles"),
    ("CIV_PERCELEN", "Percelen verdelen", "Divide parcels"),
    ("CIV_WATERDRUPPEL", "Waterdruppel volgen", "Trace a water drop"),
    ("CIV_STROOMGEBIED", "Stroomgebied bepalen", "Find a catchment"),
    ("CIV_PLASSEN", "Plassen bepalen", "Find ponds"),
    ("CIV_SPOOR", "Spooras maken", "Create a track"),
    ("CIV_WISSEL", "Wissel maken", "Create a turnout"),
    ("CIV_ZICHT", "Zichtafstand toetsen", "Check sight distance"),
    ("CIV_AS_TOETS", "As toetsen", "Check an alignment"),
    ("CIV_ONTWERPCRITERIA", "Ontwerpcriteria", "Design criteria"),
    ("CIV_MESH", "3D-mesh maken", "Create a 3D mesh"),
    ("CIV_PUNTENWOLK_TIN", "TIN uit AHN-punten", "TIN from AHN points"),
    ("CIV_PUNTENWOLK_BESTAND", "TIN uit puntenwolk", "TIN from a point cloud"),
    ("CIV_CODES_TOEPASSEN", "Codes toepassen", "Apply survey codes"),
    ("CIV_INFO", "Modelinfo", "Model info"),
    ("CIV_ACTIVITEIT_NOTEER", "Notitie", "Note"),
    ("GIS_SITE", "Projectlocatie instellen", "Set the project location"),
    ("GIS_Q_BGT", "BGT importeren", "Import the BGT"),
    ("GIS_Q_BRK", "Kadaster importeren", "Import the cadastre"),
    ("GIS_Q_LUCHTFOTO", "Luchtfoto importeren", "Import the aerial photo"),
    ("GIS_Q_KADASTER", "Kadastrale kaart importeren", "Import the cadastral map"),
    ("GIS_Q_AHN", "AHN-hoogtekaart importeren", "Import the AHN elevation map"),
    ("GIS_Q_BAG", "BAG-panden importeren", "Import BAG buildings"),
    ("GIS_Q_PERCELEN", "Percelen importeren", "Import parcels"),
    ("GIS_Q_NWB", "NWB-wegvakken importeren", "Import NWB road sections"),
    ("GIS_Q_HECTO", "Hectometerpunten importeren", "Import hectometre posts"),
    ("GIS_Q_SPOOR", "Spoorassen importeren", "Import rail centrelines"),
    ("GIS_Q_OSM", "OpenStreetMap importeren", "Import OpenStreetMap"),
    ("GIS_Q_OSMKAART", "OpenStreetMap-kaart importeren", "Import the OpenStreetMap map"),
    ("GIS_Q_SATELLIET", "Satellietbeeld importeren", "Import the satellite image"),
    ("GIS_Q_SERVER", "Kaartserver importeren", "Import a map server"),
    ("GIS_Q_SONDERINGEN", "Sonderingen en boringen importeren", "Import CPTs and boreholes"),
    ("GIS_Q_GWSW", "GWSW-riolering importeren", "Import GWSW sewers"),
    ("GIS_TEKENBLAD", "Tekenblad tekenen", "Draw a drawing sheet"),
    ("GIS_HOOGTELIJNEN", "AHN-hoogtelijnen", "AHN contours"),
    ("GIS_HOOGTEPUNTEN", "AHN-hoogtepunten", "AHN spot heights"),
    ("GIS_PUNTENWOLK", "AHN-puntenwolk", "AHN point cloud"),
    ("GIS_SERVICE_LOAD", "Dienst laden", "Load a service"),
    ("GIS_INFRA_PLACE", "Infra-object plaatsen", "Place an infrastructure object"),
    ("GIS_INFRA_UPDATE", "Infra-object bijwerken", "Update an infrastructure object"),
    ("GIS_NLCS_HERSTEL", "NLCS herstellen", "Repair NLCS"),
    ("GIS_NLCS_RAPPORT", "NLCS-controle", "NLCS check"),
    ("GIS_HOEVEELHEDEN_CSV", "Hoeveelheden exporteren", "Export quantities"),
    ("GIS_BGT_MUTATIE", "BGT-mutatie exporteren", "Export a BGT mutation"),
    ("GIS_IFC", "IFC 4.3 exporteren", "Export IFC 4.3"),
    ("GIS_PLAN_IMPORT", "Ruimtelijk plan importeren", "Import a spatial plan"),
    ("GIS_PLAN_GML", "Plan-GML importeren", "Import plan GML"),
];

/// Arguments that name what a command made.
const NAMES: &[&str] = &["NAAM", "PLAN", "DIENST", "ITEM", "INSTANCE", "RIOOL"];

/// The command in plain words: (Dutch, English).
pub fn title(command: &str) -> (String, String) {
    let (verb, rest) = command.trim().split_once(char::is_whitespace).unwrap_or((command.trim(), ""));
    let verb = verb.to_ascii_uppercase();
    let (nl, en) = TITLES.iter().find(|(v, ..)| *v == verb).map(|(_, nl, en)| (nl.to_string(), en.to_string())).unwrap_or_else(|| (verb.clone(), verb.clone()));
    let args = Args::parse(rest);
    let name = NAMES.iter().find_map(|k| args.str(k)).map(|s| s.trim_matches('"').to_string());
    match name {
        Some(n) if !n.is_empty() => (format!("{nl}: {n}"), format!("{en}: {n}")),
        _ => (nl, en),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_titles() {
        assert_eq!(title(r#"CIV_AS PI=1,2;3,4 R=50 NAAM="Rondweg Zuid""#).1, "Create an alignment: Rondweg Zuid");
        assert_eq!(title("civ_tin BRON=AHN").0, "Terrein (TIN) maken");
        assert_eq!(title("CIV_NIEUW X=1").1, "CIV_NIEUW");
    }
}
