//! Gedeelde, letterlijke invoer voor parser, voorvertoning en echte opslag.
use crate::gis::qgis::{self, Connection};

pub struct Case {
    pub connection: Connection,
    pub blocked: bool,
}

pub fn review_cases() -> Vec<Case> {
    let cases: Vec<_> = [
        include_str!("../fixtures/qgis/geoserver-authkey.xml"),
        include_str!("../fixtures/qgis/here-app-code.xml"),
    ]
    .iter()
    .flat_map(|xml| qgis::parse(xml).unwrap())
    .map(|connection| Case {
        connection,
        blocked: true,
    })
    .collect();
    assert_eq!(cases.len(), 6);
    cases
}

pub fn audited_cases() -> Vec<Case> {
    #[derive(serde::Deserialize)]
    struct Input {
        name: String,
        kind: String,
        url: String,
        blocked: bool,
    }
    let inputs: Vec<Input> =
        serde_json::from_str(include_str!("../fixtures/qgis/auth-parameters.json")).unwrap();
    assert_eq!(inputs.len(), 50);
    inputs
        .into_iter()
        .map(|input| {
            let (root, tag) = match input.kind.as_str() {
                "wms" => ("qgsWMSConnections", "wms"),
                "xyz" => ("qgsXYZTilesConnections", "xyztiles"),
                _ => panic!("onbekend verbindingstype in testinvoer"),
            };
            let attribute = |value: &str| value.replace('&', "&amp;").replace('"', "&quot;");
            let xml = format!(
                "<{root}><{tag} name=\"{}\" url=\"{}\"/></{root}>",
                attribute(&input.name),
                attribute(&input.url)
            );
            let connection = qgis::parse(&xml).unwrap().remove(0);
            Case {
                connection,
                blocked: input.blocked,
            }
        })
        .collect()
}

fn check(cases: Vec<Case>) {
    let mut failures = Vec::new();
    for case in cases {
        let c = case.connection;
        let (list, added) = qgis::merge(&[], &[&c], 888);
        let auth = c.blocking.iter().any(|reason| {
            reason == crate::lang::tr("vraagt authenticatie", "needs authentication")
        });
        let leaked = format!("{c:?}").contains("SYNTHETIC_VALUE")
            || serde_json::to_string(&list)
                .unwrap()
                .contains("SYNTHETIC_VALUE");
        let valid = if case.blocked {
            !c.importable()
                && auth
                && c.server.url.is_empty()
                && added == 0
                && list.is_empty()
                && !leaked
        } else {
            c.importable()
                && c.blocking.is_empty()
                && !c.server.url.is_empty()
                && added == 1
                && list.len() == 1
                && list[0].url == c.server.url
        };
        println!("{}: verwacht blokkade={}, importeerbaar={}, authenticatieblokkade={auth}, lege URL={}, toegevoegd={added}, gevoelige waarde bewaard={leaked}", c.server.name, case.blocked, c.importable(), c.server.url.is_empty());
        if !valid {
            failures.push(c.server.name);
        }
    }
    assert!(
        failures.is_empty(),
        "Onjuiste behandeling: {}",
        failures.join(", ")
    );
}

#[test]
fn geoserver_and_here_credentials_are_blocked() {
    check(review_cases());
}

#[test]
fn audited_credentials_and_public_parameters_are_distinguished() {
    check(audited_cases());
}
