use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn fvcs(dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("fvcs").unwrap();
    cmd.current_dir(dir);
    cmd
}

fn init_avatar(dir: &Path) {
    fs::create_dir_all(dir.join("Libs")).unwrap();
    fs::write(
        dir.join("avatar.json"),
        r#"{ "name": "TestAvatar", "authors": ["tester"] }"#,
    )
    .unwrap();
    fs::write(dir.join("Init.lua"), "local x = 1\n").unwrap();
    fs::write(dir.join("Libs/spring.lua"), "return {}\n").unwrap();
    fvcs(dir).arg("init").assert().success();
}

fn add_commit(dir: &Path, message: &str) {
    fvcs(dir).args(["add", "."]).assert().success();
    fvcs(dir).args(["commit", "-m", message]).assert().success();
}

#[test]
fn init_creates_repo() {
    let tmp = TempDir::new().unwrap();
    fvcs(tmp.path()).arg("init").assert().success();
    assert!(tmp.path().join(".fvcs/HEAD").exists());
    assert!(tmp.path().join(".fvcs/objects").is_dir());
}

#[test]
fn commit_status_log_flow() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_avatar(dir);
    add_commit(dir, "initial avatar");

    fvcs(dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("working tree clean"));

    fvcs(dir)
        .args(["log", "--oneline"])
        .assert()
        .success()
        .stdout(predicate::str::contains("initial avatar"));

    fs::write(dir.join("Init.lua"), "local x = 2\n").unwrap();
    fvcs(dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("modified:   Init.lua"));
    fvcs(dir)
        .arg("diff")
        .assert()
        .success()
        .stdout(predicate::str::contains("+local x = 2"));

    add_commit(dir, "tweak x");
    fvcs(dir)
        .args(["log", "--oneline"])
        .assert()
        .success()
        .stdout(predicate::str::contains("tweak x"))
        .stdout(predicate::str::contains("initial avatar"));
}

#[test]
fn branch_checkout_restores_bytes() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_avatar(dir);
    add_commit(dir, "v1");

    fvcs(dir).args(["branch", "feature"]).assert().success();
    fvcs(dir).args(["checkout", "feature"]).assert().success();

    fs::write(dir.join("Init.lua"), "local x = 99\n").unwrap();
    fs::remove_file(dir.join("Libs/spring.lua")).unwrap();
    fs::write(dir.join("new.lua"), "-- new\n").unwrap();
    add_commit(dir, "feature work");

    fvcs(dir).args(["checkout", "main"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.join("Init.lua")).unwrap(),
        "local x = 1\n"
    );
    assert!(dir.join("Libs/spring.lua").exists());
    assert!(!dir.join("new.lua").exists());

    fvcs(dir).args(["checkout", "feature"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.join("Init.lua")).unwrap(),
        "local x = 99\n"
    );
    assert!(dir.join("new.lua").exists());
}

#[test]
fn checkout_refused_with_uncommitted_changes() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_avatar(dir);
    add_commit(dir, "v1");
    fvcs(dir).args(["branch", "other"]).assert().success();
    fs::write(dir.join("Init.lua"), "dirty\n").unwrap();
    fvcs(dir).args(["checkout", "other"]).assert().failure();
    fvcs(dir)
        .args(["checkout", "other", "--force"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(dir.join("Init.lua")).unwrap(),
        "local x = 1\n"
    );
}

#[test]
fn restore_discards_changes() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_avatar(dir);
    add_commit(dir, "v1");
    fs::write(dir.join("Init.lua"), "broken\n").unwrap();
    fvcs(dir).args(["restore", "."]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.join("Init.lua")).unwrap(),
        "local x = 1\n"
    );
}

#[test]
fn tag_and_prefix_resolve() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_avatar(dir);
    add_commit(dir, "v1");
    fvcs(dir).args(["tag", "release-1"]).assert().success();
    fs::write(dir.join("Init.lua"), "local x = 2\n").unwrap();
    add_commit(dir, "v2");

    fvcs(dir).args(["checkout", "release-1"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.join("Init.lua")).unwrap(),
        "local x = 1\n"
    );
}

#[test]
fn fvcsignore_excludes_files() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();
    init_avatar(dir);
    fs::write(dir.join(".fvcsignore"), "*.tmp\ncache/\n").unwrap();
    fs::write(dir.join("notes.tmp"), "junk").unwrap();
    fs::create_dir_all(dir.join("cache")).unwrap();
    fs::write(dir.join("cache/blob.bin"), [1u8, 2, 3]).unwrap();
    fvcs(dir).args(["add", "."]).assert().success();
    fvcs(dir)
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("notes.tmp").not())
        .stdout(predicate::str::contains("blob.bin").not());
}
