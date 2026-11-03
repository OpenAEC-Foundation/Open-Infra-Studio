//! Overlappende opslag in een eigen proces en tijdelijke configuratiemap.
use std::sync::{Arc, Barrier};

use crate::gis::servers::{self, MapServer};

#[test]
fn overlapping_saves_keep_complete_json() {
    let directory = std::env::temp_dir().join(format!(
        "infra-server-saves-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "gis::servers::storage_tests::server_save_worker",
            "--nocapture",
        ])
        .env("INFRA_SERVER_SAVE_WORKER", "1")
        .env("XDG_CONFIG_HOME", &directory)
        .env("APPDATA", &directory)
        .output()
        .unwrap();
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
    let (failed, invalid) = std::thread::scope(|scope| {
        let run = |server: MapServer| {
            let mut failed = 0;
            let mut invalid = 0;
            for _ in 0..128 {
                barrier.wait();
                failed += usize::from(servers::save_user(std::slice::from_ref(&server)).is_err());
                barrier.wait();
                let json =
                    std::fs::read_to_string(servers::config_dir().join("servers.json")).unwrap();
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
        let one = scope.spawn(|| run(a.clone()));
        let two = scope.spawn(|| run(b.clone()));
        let (a, b) = (one.join().unwrap(), two.join().unwrap());
        (a.0 + b.0, a.1 + b.1)
    });
    println!("256 overlappende saves: {failed} opslagfouten, {invalid} onvolledige of ongeldige JSON-controles");
    assert_eq!((failed, invalid), (0, 0));
}
