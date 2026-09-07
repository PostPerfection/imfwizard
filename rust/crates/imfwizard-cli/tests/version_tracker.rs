use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use std::sync::OnceLock;
use tempfile::TempDir;

fn cmd() -> Command {
    static CONFIG_DIRECTORY: OnceLock<TempDir> = OnceLock::new();
    let directory = CONFIG_DIRECTORY.get_or_init(|| TempDir::new().unwrap());
    let mut command = Command::cargo_bin("imfwizard").unwrap();
    command.env("XDG_CONFIG_HOME", directory.path());
    command
}

fn record(db: &Path, package_uuid: &str, title: &str, destination: &str) {
    cmd()
        .args([
            "version",
            "record",
            "--db",
            db.to_str().unwrap(),
            "--package-uuid",
            package_uuid,
            "--title",
            title,
            "--version",
            "OV",
            "--destination",
            destination,
            "--method",
            "hard_drive",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(package_uuid));
}

#[test]
fn recorded_deliveries_are_listed_filtered_and_exported() {
    let directory = TempDir::new().unwrap();
    let db = directory.path().join("deliveries.db");

    record(&db, "imp-uuid-1", "First Feature", "Cinema Chain A");
    record(&db, "imp-uuid-2", "Second Feature", "Cinema Chain B");

    cmd()
        .args(["version", "list", "--db", db.to_str().unwrap()])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("imp-uuid-1")
                .and(predicate::str::contains("imp-uuid-2"))
                .and(predicate::str::contains("Cinema Chain A")),
        );

    cmd()
        .args([
            "version",
            "list",
            "--db",
            db.to_str().unwrap(),
            "--destination",
            "Cinema Chain B",
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("imp-uuid-2")
                .and(predicate::str::contains("imp-uuid-1").not()),
        );

    let exported = directory.path().join("history.json");
    cmd()
        .args([
            "version",
            "export",
            "--db",
            db.to_str().unwrap(),
            "--output",
            exported.to_str().unwrap(),
        ])
        .assert()
        .success();

    let history = std::fs::read_to_string(&exported).unwrap();
    assert!(
        history.contains("First Feature") && history.contains("Second Feature"),
        "the export is missing a delivery: {history}"
    );
}
