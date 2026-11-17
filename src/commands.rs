//! Command names. The English name is the command's name: the ribbon, help,
//! documentation, MCP server and messages use it. Older names (mostly Dutch)
//! keep working as aliases, so existing scripts, macros and agents do not
//! break. Both plugins put every command line through [`normalise`] before
//! anything reads it, so the rest of the code only sees the English name.

/// `(alias, name)`: an older name and the command it runs. An alias keeps
/// its plugin's prefix, because the host routes by prefix before the plugin
/// sees the command.
pub const ALIASES: &[(&str, &str)] = &[
    // GIS tab.
    ("GIS_BESTAND", "GIS_FILE"),
    ("GIS_BESTAND_IMPORT", "GIS_FILE_IMPORT"),
    ("GIS_BGT_BIJWERKEN", "GIS_BGT_UPDATE"),
    ("GIS_BGT_MUTATIE", "GIS_BGT_MUTATION"),
    ("GIS_BIBLIOTHEEK", "GIS_LIBRARY"),
    ("GIS_DIENSTEN", "GIS_SERVICES"),
    ("GIS_DIENST_LADEN", "GIS_SERVICE_LOAD"),
    ("GIS_GEBIED", "GIS_AREA"),
    ("GIS_HOEVEELHEDEN", "GIS_QUANTITIES"),
    ("GIS_HOEVEELHEDEN_CSV", "GIS_QUANTITIES_CSV"),
    ("GIS_HOEVEELHEDEN_TABEL", "GIS_QUANTITIES_TABLE"),
    ("GIS_HOOGTELIJNEN", "GIS_CONTOURS"),
    ("GIS_HOOGTEPUNTEN", "GIS_SPOT_HEIGHTS"),
    ("GIS_INFRA_BIBLIOTHEEK", "GIS_INFRA_LIBRARY"),
    ("GIS_INFRA_CATALOGUS", "GIS_INFRA_CATALOG"),
    ("GIS_INFRA_PLAATS", "GIS_INFRA_PLACE"),
    ("GIS_KAART", "GIS_WMS"),
    ("GIS_KLIC_AANVRAAG", "GIS_KLIC_REQUEST"),
    ("GIS_KLIC_MAP", "GIS_KLIC_FOLDER"),
    ("GIS_KLIC_VOLGEN", "GIS_KLIC_WATCH"),
    ("GIS_LOCALISATIE", "GIS_LOCALISATION"),
    ("GIS_LOCATIE", "GIS_LOCATION"),
    ("GIS_WERKBANK", "GIS_LOCATION"),
    ("GIS_NLCS", "GIS_BGT"),
    ("GIS_NLCSPP_CONTROLE", "GIS_NLCSPP_CHECK"),
    ("GIS_NLCS_CONTROLE", "GIS_NLCS_CHECK"),
    ("GIS_NLCS_HERSTEL", "GIS_NLCS_REPAIR"),
    ("GIS_NLCS_RAPPORT", "GIS_NLCS_REPORT"),
    ("GIS_PARKEERVAKKEN", "GIS_PARKING"),
    ("GIS_PLANNEN", "GIS_PLANS"),
    ("GIS_PROFIEL", "GIS_PROFILE"),
    ("GIS_PUNT", "GIS_POINT_INFO"),
    ("GIS_PUNTENWOLK", "GIS_POINT_CLOUD"),
    ("GIS_GWSW", "GIS_Q_GWSW"),
    ("GIS_Q_BODEMKAART", "GIS_Q_SOIL_MAP"),
    ("GIS_Q_GEOMORFOLOGIE", "GIS_Q_GEOMORPHOLOGY"),
    ("GIS_Q_KADASTER", "GIS_Q_CADASTRE"),
    ("GIS_Q_LUCHTFOTO", "GIS_Q_AERIAL"),
    ("GIS_Q_OSMKAART", "GIS_Q_OSM_MAP"),
    ("GIS_Q_PERCELEN", "GIS_Q_PARCELS"),
    ("GIS_Q_SATELLIET", "GIS_Q_SATELLITE"),
    ("GIS_Q_SONDERINGEN", "GIS_Q_CPT"),
    ("GIS_SONDERINGEN", "GIS_Q_CPT"),
    ("GIS_Q_SPOOR", "GIS_Q_RAIL"),
    ("GIS_SWEPTPATH", "GIS_SWEPT_PATH"),
    ("GIS_VEEGPAD", "GIS_SWEPT_PATH"),
    ("GIS_TEKENBLAD", "GIS_SHEET"),
    ("GIS_VOERTUIG", "GIS_VEHICLE"),
    ("GIS_VOERTUIGEN", "GIS_VEHICLES"),
    ("GIS_VOERTUIGOVERZICHT", "GIS_VEHICLE_CATALOG"),
    ("GIS_VEHICLE_CATALOGUE", "GIS_VEHICLE_CATALOG"),
    // Civiel tab.
    ("CIV_ANALYSE", "CIV_ELEVATION_ANALYSIS"),
    ("CIV_AS", "CIV_ALIGNMENT"),
    ("CIV_AS_OFFSET", "CIV_ALIGNMENT_OFFSET"),
    ("CIV_AS_TOETS", "CIV_ALIGNMENT_CHECK"),
    ("CIV_BIJWERKEN", "CIV_PROFILES_UPDATE"),
    ("CIV_BLADEN", "CIV_SHEETS"),
    ("CIV_BLADEN_PDF", "CIV_SHEETS_PDF"),
    ("CIV_CODES_TOEPASSEN", "CIV_CODES_APPLY"),
    ("CIV_DOORZICHT", "CIV_SEE_THROUGH"),
    ("CIV_DWARSPROFIELEN", "CIV_CROSS_SECTIONS"),
    ("CIV_FOTOTERREIN", "CIV_AERIAL_TERRAIN"),
    ("CIV_GRONDBALANS", "CIV_MASS_HAUL"),
    ("CIV_GRONDPROFIEL", "CIV_SOIL_PROFILE"),
    ("CIV_GRONDSOORTEN", "CIV_SOIL_UNITS"),
    ("CIV_HOOGTELIJNEN", "CIV_CONTOURS"),
    ("CIV_INSTELLINGEN", "CIV_SETTINGS"),
    ("CIV_KANTLIJN", "CIV_FEATURE_LINE"),
    ("CIV_KANTLIJN_HELLING", "CIV_FEATURE_LINE_GRADE"),
    ("CIV_KANTLIJN_PEIL", "CIV_FEATURE_LINE_LEVEL"),
    ("CIV_KANTLIJN_PUNT", "CIV_FEATURE_LINE_POINT"),
    ("CIV_KANTLIJN_VERHOOG", "CIV_FEATURE_LINE_RAISE"),
    ("CIV_KRUISPUNT", "CIV_JUNCTION"),
    ("CIV_LABELS_BIJWERKEN", "CIV_LABELS_UPDATE"),
    ("CIV_LENGTEPROFIEL", "CIV_LONG_PROFILE"),
    ("CIV_ONTWERPCRITERIA", "CIV_DESIGN_CRITERIA"),
    ("CIV_ONTWERPPROFIEL", "CIV_DESIGN_PROFILE"),
    ("CIV_PERCEEL_INFO", "CIV_PARCEL_INFO"),
    ("CIV_PERCELEN", "CIV_PARCELS"),
    ("CIV_PLASSEN", "CIV_PONDS"),
    ("CIV_PROFIELTYPE", "CIV_ASSEMBLY"),
    ("CIV_PROFIELTYPEN", "CIV_ASSEMBLIES"),
    ("CIV_PUNT", "CIV_POINT"),
    ("CIV_PUNTENWOLK_BESTAND", "CIV_POINT_CLOUD_FILE"),
    ("CIV_PUNTENWOLK_TIN", "CIV_POINT_CLOUD_TIN"),
    ("CIV_PUNTEN_EXPORT", "CIV_POINTS_EXPORT"),
    ("CIV_PUNTEN_IMPORT", "CIV_POINTS_IMPORT"),
    ("CIV_RIOOL", "CIV_SEWER"),
    ("CIV_RIOOL_BIJWERKEN", "CIV_SEWER_UPDATE"),
    ("CIV_RIOOL_EXPORT", "CIV_SEWER_EXPORT"),
    ("CIV_RIOOL_GWSW", "CIV_SEWER_GWSW"),
    ("CIV_RIOOL_HELP", "CIV_SEWER_HELP"),
    ("CIV_RIOOL_IMPORT", "CIV_SEWER_IMPORT"),
    ("CIV_RIOOL_PROFIEL", "CIV_SEWER_PROFILE"),
    ("CIV_RIOOL_TOETS", "CIV_SEWER_CHECK"),
    ("CIV_ROTONDE", "CIV_ROUNDABOUT"),
    ("CIV_SONDERINGEN3D", "CIV_CPT3D"),
    ("CIV_ONDERGROND3D", "CIV_CPT3D"),
    ("CIV_SPOOR", "CIV_TRACK"),
    ("CIV_STABILITEIT", "CIV_SLOPE_STABILITY"),
    ("CIV_STROOMGEBIED", "CIV_CATCHMENT"),
    ("CIV_TALUD", "CIV_SLOPE"),
    ("CIV_WATERDRUPPEL", "CIV_WATER_DROP"),
    ("CIV_WATERPEIL", "CIV_WATER_LEVEL"),
    ("CIV_WISSEL", "CIV_TURNOUT"),
    ("CIV_ZETTING", "CIV_SETTLEMENT"),
    ("CIV_ZICHT", "CIV_SIGHT_DISTANCE"),
];

/// The command `verb` (uppercase) runs: its English name.
pub fn canonical(verb: &str) -> &str {
    ALIASES.iter().find(|(alias, _)| *alias == verb).map_or(verb, |(_, name)| name)
}

/// `cmd` with its first word replaced by the command's name, uppercase; the
/// arguments keep their case (paths).
pub fn normalise(cmd: &str) -> String {
    let cmd = cmd.trim_start();
    let (verb, rest) = cmd.split_at(cmd.find(char::is_whitespace).unwrap_or(cmd.len()));
    format!("{}{rest}", canonical(&verb.to_ascii_uppercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    const PREFIXES: [&str; 2] = ["GIS_", "CIV_"];

    fn prefix(name: &str) -> Option<&'static str> {
        PREFIXES.into_iter().find(|p| name.starts_with(p))
    }

    #[test]
    fn an_alias_runs_its_english_command_and_keeps_the_arguments() {
        assert_eq!(normalise("gis_gebied"), "GIS_AREA");
        assert_eq!(normalise("CIV_DWARSPROFIELEN AS=1A OM=20"), "CIV_CROSS_SECTIONS AS=1A OM=20");
        assert_eq!(normalise("  GIS_Bestand \"C:/Data/Kaart.geojson\""), "GIS_FILE \"C:/Data/Kaart.geojson\"");
        assert_eq!(normalise("GIS_AREA"), "GIS_AREA");
        assert_eq!(normalise("CIV_TIN BRON=AHN"), "CIV_TIN BRON=AHN");
    }

    #[test]
    fn no_two_commands_share_a_name() {
        let mut seen = std::collections::HashSet::new();
        for (alias, name) in ALIASES {
            assert!(seen.insert(*alias), "{alias} staat twee keer in de tabel");
            assert_ne!(alias, name, "{alias} wijst naar zichzelf");
            assert!(ALIASES.iter().all(|(a, _)| a != name), "{name} is zowel naam als alias");
            assert_eq!(prefix(alias), prefix(name), "{alias} → {name}: de host stuurt op voorvoegsel");
            assert!(prefix(alias).is_some(), "{alias}: geen GIS_- of CIV_-voorvoegsel");
        }
    }

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    }

    /// Text files of the repository, without build output and history.
    fn files(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !matches!(name.as_str(), "target" | ".git" | "node_modules" | "__pycache__") {
                    files(&path, out);
                }
            } else if matches!(path.extension().and_then(|e| e.to_str()), Some("rs" | "py" | "md" | "json" | "jsonl" | "toml" | "yml" | "txt"))
                // The release notes and known bugs record history in the names of the time.
                && !matches!(name.as_str(), "RELEASE_NOTES.md" | "knownbugs.md")
                && path != root().join("src/commands.rs")
            {
                out.push(path);
            }
        }
    }

    /// Whether `text` names `word` as a whole command name.
    fn names(text: &str, word: &str) -> bool {
        let part = |c: char| c.is_ascii_alphanumeric() || c == '_';
        text.match_indices(word).any(|(i, _)| {
            !text[..i].chars().next_back().is_some_and(part) && !text[i + word.len()..].chars().next().is_some_and(part)
        })
    }

    #[test]
    fn code_and_documentation_use_the_english_names() {
        let mut all = Vec::new();
        files(&root(), &mut all);
        assert!(all.len() > 50, "repository niet gevonden: {}", root().display());
        let mut found = Vec::new();
        for path in &all {
            let Ok(text) = std::fs::read_to_string(path) else { continue };
            for (alias, name) in ALIASES {
                if names(&text, alias) {
                    found.push(format!("{}: {alias} → {name}", path.strip_prefix(root()).unwrap().display()));
                }
            }
        }
        assert!(found.is_empty(), "oude namen buiten de aliastabel:\n{}", found.join("\n"));
    }

    #[test]
    fn every_alias_reaches_a_command_the_plugin_handles() {
        let mut code = String::new();
        let mut all = Vec::new();
        files(&root().join("src"), &mut all);
        files(&root().join("civiel/src"), &mut all);
        for path in all.iter().filter(|p| p.extension().is_some_and(|e| e == "rs")) {
            code.push_str(&std::fs::read_to_string(path).unwrap());
        }
        for (alias, name) in ALIASES {
            assert!(code.contains(&format!("\"{name}\"")), "{alias} → {name}, maar geen plugin behandelt {name}");
        }
    }

    #[test]
    fn the_dutch_command_names_are_aliases() {
        // A sample over both plugins; the table itself lists them all.
        for (alias, name) in [
            ("GIS_LOCATIE", "GIS_LOCATION"),
            ("GIS_GEBIED", "GIS_AREA"),
            ("GIS_Q_LUCHTFOTO", "GIS_Q_AERIAL"),
            ("GIS_BESTAND", "GIS_FILE"),
            ("CIV_AS", "CIV_ALIGNMENT"),
            ("CIV_DWARSPROFIELEN", "CIV_CROSS_SECTIONS"),
            ("CIV_GRONDBALANS", "CIV_MASS_HAUL"),
        ] {
            assert_eq!(canonical(alias), name);
            assert_eq!(canonical(name), name);
        }
    }
}
