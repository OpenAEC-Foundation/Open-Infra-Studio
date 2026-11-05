//! Overlappende opslag in een eigen proces en tijdelijke configuratiemap.
use std::sync::{Arc, Barrier};

use crate::gis::servers::{self, MapServer};

fn directory() -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "infra-server-saves-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn worker(directory: &std::path::Path, test: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            &format!("gis::servers::storage_tests::{test}"),
            "--nocapture",
        ])
        .env("INFRA_SERVER_SAVE_WORKER", "1")
        .env("XDG_CONFIG_HOME", directory)
        .env("APPDATA", directory);
    command
}

#[test]
fn overlapping_saves_keep_complete_json() {
    let directory = directory();
    let result = worker(&directory, "server_save_worker").output().unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}

#[test]
fn server_save_worker() {
    if std::env::var_os("INFRA_SERVER_SAVE_WORKER").is_none() {
        return;
    }
    let mut a = servers::presets().remove(0);
    a.builtin = false;
    a.id = "save-a".into();
    a.attribution = "a".repeat(256 * 1024);
    let mut b = a.clone();
    b.id = "save-b".into();
    b.attribution = "b".repeat(256 * 1024);
    let barrier = Arc::new(Barrier::new(2));
    let run = |server: MapServer| {
        let mut failed = 0;
        let mut invalid = 0;
        for _ in 0..128 {
            barrier.wait();
            failed += usize::from(servers::save_user(std::slice::from_ref(&server)).is_err());
            barrier.wait();
            let json = std::fs::read_to_string(servers::config_dir().join("servers.json")).unwrap();
            let valid = serde_json::from_str::<Vec<MapServer>>(&json).is_ok_and(|list| {
                list.len() == 1
                    && ((list[0].id == "save-a" && list[0].attribution == a.attribution)
                        || (list[0].id == "save-b" && list[0].attribution == b.attribution))
            });
            invalid += usize::from(!valid);
            barrier.wait();
        }
        (failed, invalid)
    };
    let (failed, invalid) = std::thread::scope(|scope| {
        let one = scope.spawn(|| run(a.clone()));
        let two = scope.spawn(|| run(b.clone()));
        let (a, b) = (one.join().unwrap(), two.join().unwrap());
        (a.0 + b.0, a.1 + b.1)
    });
    println!("256 overlappende saves: {failed} opslagfouten, {invalid} onvolledige of ongeldige JSON-controles");
    assert_eq!((failed, invalid), (0, 0));

    let path = servers::config_dir().join("servers.json");
    std::fs::write(&path, b"{invalid").unwrap();
    assert!(servers::update_user(|user| user.clear()).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"{invalid");
}

#[test]
fn concurrent_process_imports_preserve_all_servers() {
    let directory = directory();
    let spawn = |name| {
        worker(&directory, "server_update_worker")
            .env("INFRA_SERVER_UPDATE_WORKER", name)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    };
    let a = spawn("a");
    let b = spawn("b");
    for child in [a, b] {
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let json =
        std::fs::read_to_string(directory.join("OpenCADStudio/open-infra/servers.json")).unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    let list: Vec<MapServer> = serde_json::from_str(&json).unwrap();
    assert_eq!(
        list.len(),
        32,
        "geen van beide processen mag toevoegingen verliezen"
    );
    let ids: std::collections::HashSet<_> =
        list.iter().map(|s| s.id.to_ascii_lowercase()).collect();
    assert_eq!(ids.len(), list.len());
}

#[test]
fn server_update_worker() {
    let Ok(name) = std::env::var("INFRA_SERVER_UPDATE_WORKER") else {
        return;
    };
    for i in 0..16 {
        let connections = crate::gis::qgis::parse(&format!(r#"<qgsWMSConnections><wms name="Openbaar" url="https://{name}.invalid/wms?map={i}"/></qgsWMSConnections>"#)).unwrap();
        let added = servers::update_user(|user| {
            let (list, added) = crate::gis::qgis::merge(user, &[&connections[0]], 888);
            *user = list;
            added
        })
        .unwrap();
        assert_eq!(added, 1);
    }
}

#[test]
fn azure_maps_subscription_keys_do_not_reach_servers_json() {
    let directory = directory();
    let result = worker(&directory, "azure_maps_storage_worker")
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn azure_maps_storage_worker() {
    if std::env::var_os("INFRA_SERVER_SAVE_WORKER").is_none() {
        return;
    }
    let connections =
        crate::gis::qgis::parse(include_str!("../fixtures/qgis/azure-maps.xml")).unwrap();
    assert_eq!(connections.len(), 3);
    let mut failures = Vec::new();
    for c in &connections {
        let added = servers::update_user(|user| {
            let (list, added) = crate::gis::qgis::merge(user, &[c], 888);
            *user = list;
            added
        })
        .unwrap();
        let json = std::fs::read_to_string(servers::config_dir().join("servers.json")).unwrap();
        let leaked = json.contains("SYNTHETIC_VALUE");
        println!(
            "{}: toegevoegd={added}, gevoelige waarde in servers.json={leaked}",
            c.server.name
        );
        if added != 0 || leaked {
            failures.push(c.server.name.clone());
        }
    }
    assert!(
        failures.is_empty(),
        "Opgeslagen in servers.json: {}",
        failures.join(", ")
    );
}

fn check_auth_storage(group: &str) {
    let directory = directory();
    let result = worker(&directory, "auth_credentials_storage_worker")
        .env("INFRA_AUTH_CASE_GROUP", group)
        .output()
        .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
    assert!(
        result.status.success(),
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn ordnance_survey_keys_do_not_reach_servers_json() {
    check_auth_storage("os");
}

#[test]
fn geoserver_and_here_credentials_do_not_reach_servers_json() {
    check_auth_storage("review");
}

#[test]
fn audited_credentials_and_public_urls_are_saved_correctly() {
    check_auth_storage("audit");
}

#[test]
fn auth_credentials_storage_worker() {
    let Ok(group) = std::env::var("INFRA_AUTH_CASE_GROUP") else {
        return;
    };
    let cases = match group.as_str() {
        "os" => crate::gis::qgis::auth_test_support::os_cases(),
        "review" => crate::gis::qgis::auth_test_support::review_cases(),
        "audit" => crate::gis::qgis::auth_test_support::audited_cases(),
        _ => panic!("onbekende groep testinvoer"),
    };
    let mut failures = Vec::new();
    for case in cases {
        // Iedere invoer begint met een eigen lege lijst, zodat een eerder
        // gelekte waarde de uitkomst van de volgende invoer niet bepaalt.
        servers::save_user(&[]).unwrap();
        let c = case.connection;
        let added = servers::update_user(|user| {
            let (list, added) = crate::gis::qgis::merge(user, &[&c], 888);
            *user = list;
            added
        })
        .unwrap();
        let json = std::fs::read_to_string(servers::config_dir().join("servers.json")).unwrap();
        let list: Vec<MapServer> = serde_json::from_str(&json).unwrap();
        let leaked = crate::gis::qgis::auth_test_support::contains_test_secret(&json);
        let valid = if case.blocked {
            added == 0 && list.is_empty() && !leaked
        } else {
            added == 1
                && list.len() == 1
                && !c.server.url.is_empty()
                && list[0].url == c.server.url
                && !leaked
        };
        println!("{}: verwacht blokkade={}, toegevoegd={added}, gevoelige waarde in servers.json={leaked}", c.server.name, case.blocked);
        if !valid {
            failures.push(c.server.name);
        }
    }
    assert!(
        failures.is_empty(),
        "Onjuiste opslag: {}",
        failures.join(", ")
    );
}
