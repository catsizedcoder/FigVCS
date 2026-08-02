use std::fs;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcCommand};
use std::time::{Duration, Instant};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn fvcs(dir: &Path, home: &Path) -> Command {
    let mut cmd = Command::cargo_bin("fvcs").unwrap();
    cmd.current_dir(dir);
    cmd.env("USERPROFILE", home);
    cmd.env("HOME", home);
    cmd
}

fn server_bin() -> PathBuf {
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    path.pop();
    path.push("fvcs-server.exe");
    path
}

struct Server {
    child: Child,
    port: u16,
    dir: PathBuf,
}

impl Server {
    fn start(dir: &Path) -> Server {
        Self::start_with(dir, &[])
    }

    fn start_with(dir: &Path, extra: &[&str]) -> Server {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let mut cmd = ProcCommand::new(server_bin());
        cmd.arg("--dir")
            .arg(dir)
            .arg("--port")
            .arg(port.to_string())
            .args(extra)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut child = cmd.spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Server {
                    child,
                    port,
                    dir: dir.to_path_buf(),
                };
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        panic!("server did not start");
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn repo_url(&self, repo: &str) -> String {
        format!("{}/{repo}", self.base())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn init_avatar(dir: &Path, home: &Path, name: &str, description: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("avatar.json"),
        format!(r#"{{ "name": "{name}", "description": "{description}", "authors": ["tester"] }}"#),
    )
    .unwrap();
    fs::write(dir.join("Init.lua"), "local x = 1\n").unwrap();
    fvcs(dir, home).arg("init").assert().success();
    fvcs(dir, home).args(["add", "."]).assert().success();
    fvcs(dir, home)
        .args(["commit", "-m", "v1"])
        .assert()
        .success();
}

fn login(home: &Path, server: &Server, username: &str) {
    fvcs(home, home)
        .args([
            "login",
            &server.base(),
            "--register",
            "-u",
            username,
            "-p",
            "hunter2x",
        ])
        .assert()
        .success();
}

#[test]
fn push_clone_readme_with_accounts() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let server = Server::start(&tmp.path().join("srv"));

    let alice = tmp.path().join("alice");
    init_avatar(&alice, &home, "CoolAvatar", "A very cool avatar");
    login(&home, &server, "alice");
    fvcs(&alice, &home)
        .args(["remote", "add", "origin", &server.repo_url("cool-avatar")])
        .assert()
        .success();
    fvcs(&alice, &home)
        .args(["push"])
        .assert()
        .success()
        .stdout(predicate::str::contains("README.md generated"));

    let readme = fs::read_to_string(server.dir.join("cool-avatar/README.md")).unwrap();
    assert!(readme.contains("# CoolAvatar"));
    assert!(readme.contains("A very cool avatar"));

    let bob = tmp.path().join("bob");
    fvcs(&alice, &home)
        .args(["remote", "visibility", "origin", "public"])
        .assert()
        .success();
    fvcs(tmp.path(), &home)
        .args([
            "clone",
            &server.repo_url("cool-avatar"),
            bob.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(bob.join("Init.lua")).unwrap(),
        "local x = 1\n"
    );
}

#[test]
fn private_by_default_and_sharing() {
    let tmp = TempDir::new().unwrap();
    let home_a = tmp.path().join("home-a");
    let home_b = tmp.path().join("home-b");
    fs::create_dir_all(&home_a).unwrap();
    fs::create_dir_all(&home_b).unwrap();
    let server = Server::start(&tmp.path().join("srv"));

    let alice_repo = tmp.path().join("alice");
    init_avatar(&alice_repo, &home_a, "Private", "alice's stuff");
    login(&home_a, &server, "alice");
    fvcs(&alice_repo, &home_a)
        .args(["remote", "add", "origin", &server.repo_url("private-repo")])
        .assert()
        .success();
    fvcs(&alice_repo, &home_a).args(["push"]).assert().success();

    login(&home_b, &server, "bob");
    let bob_clone = tmp.path().join("bob-clone");
    fvcs(tmp.path(), &home_b)
        .args([
            "clone",
            &server.repo_url("private-repo"),
            bob_clone.to_str().unwrap(),
        ])
        .assert()
        .failure();

    let anon_clone = tmp.path().join("anon");
    fvcs(tmp.path(), &home_b)
        .args([
            "clone",
            &server.repo_url("private-repo"),
            anon_clone.to_str().unwrap(),
        ])
        .assert()
        .failure();

    let bob_repo = tmp.path().join("bob");
    init_avatar(&bob_repo, &home_b, "Bob", "bob's stuff");
    fvcs(&bob_repo, &home_b)
        .args(["remote", "add", "origin", &server.repo_url("private-repo")])
        .assert()
        .success();
    fvcs(&bob_repo, &home_b).args(["push"]).assert().failure();

    fvcs(&alice_repo, &home_a)
        .args(["share", "bob"])
        .assert()
        .success();
    let bob_clone = tmp.path().join("bob-work");
    fvcs(tmp.path(), &home_b)
        .args([
            "clone",
            &server.repo_url("private-repo"),
            bob_clone.to_str().unwrap(),
        ])
        .assert()
        .success();
    fs::write(bob_clone.join("Init.lua"), "from bob\n").unwrap();
    fvcs(&bob_clone, &home_b)
        .args(["add", "."])
        .assert()
        .success();
    fvcs(&bob_clone, &home_b)
        .args(["commit", "-m", "bob was here"])
        .assert()
        .success();
    fvcs(&bob_clone, &home_b).args(["push"]).assert().success();

    fvcs(&alice_repo, &home_a)
        .args(["share", "bob", "--remove"])
        .assert()
        .success();
    fs::write(bob_clone.join("Init.lua"), "bob again\n").unwrap();
    fvcs(&bob_clone, &home_b)
        .args(["add", "."])
        .assert()
        .success();
    fvcs(&bob_clone, &home_b)
        .args(["commit", "-m", "bob strikes back"])
        .assert()
        .success();
    fvcs(&bob_clone, &home_b).args(["push"]).assert().failure();
}

#[test]
fn login_wrong_password_fails() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let server = Server::start(&tmp.path().join("srv"));
    login(&home, &server, "carol");
    fvcs(&home, &home)
        .args(["login", &server.base(), "-u", "carol", "-p", "wrongpass"])
        .assert()
        .failure();
}

#[test]
fn registry_sync_respects_local_edits() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let server = Server::start(&tmp.path().join("srv"));
    login(&home, &server, "libowner");

    let libsrc = tmp.path().join("mylib-src");
    fs::create_dir_all(&libsrc).unwrap();
    fvcs(&libsrc, &home).arg("init").assert().success();
    fs::write(libsrc.join("cool.lua"), "v1\n").unwrap();
    fvcs(&libsrc, &home).args(["add", "."]).assert().success();
    fvcs(&libsrc, &home)
        .args(["commit", "-m", "lib v1"])
        .assert()
        .success();
    fvcs(&libsrc, &home)
        .args(["remote", "add", "origin", &server.repo_url("mylib")])
        .assert()
        .success();
    fvcs(&libsrc, &home)
        .args(["push", "--no-readme"])
        .assert()
        .success();
    fvcs(&libsrc, &home)
        .args(["remote", "visibility", "origin", "public"])
        .assert()
        .success();

    let lib_tree_hash = {
        let repo = fvcs::repo::Repository::discover(&libsrc).unwrap();
        let tip = fvcs::refs::current_commit(&repo).unwrap().unwrap();
        let commit = fvcs::commit::Commit::load(&repo, &tip).unwrap();
        fvcs::registry::lib_hash(&commit.tree)
    };
    let registry = serde_json::json!({
        "updated": "2026-08-01",
        "libs": [{
            "name": "MyLib",
            "source": server.repo_url("mylib"),
            "hashes": [lib_tree_hash]
        }]
    });
    fs::write(
        server.dir.join("registry.json"),
        serde_json::to_string_pretty(&registry).unwrap(),
    )
    .unwrap();

    let avatar = tmp.path().join("avatar");
    init_avatar(&avatar, &home, "User", "uses libs");
    fvcs(&avatar, &home)
        .args(["registry-url", &server.base()])
        .assert()
        .success();
    fvcs(&avatar, &home).args(["sync"]).assert().success();
    assert_eq!(
        fs::read_to_string(avatar.join("MyLib/cool.lua")).unwrap(),
        "v1\n"
    );

    fs::write(avatar.join("MyLib/cool.lua"), "my edit\n").unwrap();
    fvcs(&avatar, &home)
        .args(["sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Keeping your modified 'MyLib'"));
    assert_eq!(
        fs::read_to_string(avatar.join("MyLib/cool.lua")).unwrap(),
        "my edit\n"
    );

    fvcs(&avatar, &home)
        .args(["sync", "--force"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(avatar.join("MyLib/cool.lua")).unwrap(),
        "v1\n"
    );
}

#[test]
fn push_no_readme_is_remembered() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let server = Server::start(&tmp.path().join("srv"));
    login(&home, &server, "dana");

    let alice = tmp.path().join("alice");
    init_avatar(&alice, &home, "NoReadme", "skip readme");
    fvcs(&alice, &home)
        .args(["remote", "add", "origin", &server.repo_url("noreadme")])
        .assert()
        .success();
    fvcs(&alice, &home)
        .args(["push", "--no-readme"])
        .assert()
        .success();
    assert!(!server.dir.join("noreadme/README.md").exists());
    assert!(fs::read_to_string(alice.join(".fvcs/config.json"))
        .unwrap()
        .contains("\"no_readme\": true"));

    fs::write(alice.join("Init.lua"), "local x = 2\n").unwrap();
    fvcs(&alice, &home).args(["add", "."]).assert().success();
    fvcs(&alice, &home)
        .args(["commit", "-m", "v2"])
        .assert()
        .success();
    fvcs(&alice, &home).args(["push"]).assert().success();
    assert!(!server.dir.join("noreadme/README.md").exists());
}

#[test]
fn registration_is_rate_limited() {
    let tmp = TempDir::new().unwrap();
    let server = Server::start_with(&tmp.path().join("srv"), &["--pol-difficulty", "10"]);

    for i in 0..5 {
        let result = fvcs::http_store::HttpStore::register(
            &server.base(),
            &format!("bot{i}"),
            "password123",
        );
        assert!(
            result.is_ok(),
            "registration {i} should succeed: {result:?}"
        );
    }
    let blocked = fvcs::http_store::HttpStore::register(&server.base(), "bot6", "password123");
    assert!(blocked.is_err());
    assert!(blocked.unwrap_err().to_string().contains("rate-limiting"));
}

#[test]
fn push_quota_is_enforced() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let server = Server::start_with(
        &tmp.path().join("srv"),
        &["--pol-difficulty", "10", "--max-pushes-per-day", "1"],
    );
    login(&home, &server, "quotauser");

    let repo = tmp.path().join("repo");
    init_avatar(&repo, &home, "Quota", "quota test");
    fvcs(&repo, &home)
        .args(["remote", "add", "origin", &server.repo_url("quota")])
        .assert()
        .success();
    fvcs(&repo, &home).args(["push"]).assert().success();

    fs::write(repo.join("Init.lua"), "local x = 2\n").unwrap();
    fvcs(&repo, &home).args(["add", "."]).assert().success();
    fvcs(&repo, &home)
        .args(["commit", "-m", "v2"])
        .assert()
        .success();
    fvcs(&repo, &home)
        .args(["push"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("rate-limiting"));
}

#[test]
fn repo_size_limit_is_enforced() {
    let tmp = TempDir::new().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let server = Server::start_with(&tmp.path().join("srv"), &["--max-repo-size-mb", "1"]);
    login(&home, &server, "erin");

    let repo = tmp.path().join("repo");
    init_avatar(&repo, &home, "Sized", "size test");
    fvcs(&repo, &home)
        .args(["remote", "add", "origin", &server.repo_url("sized")])
        .assert()
        .success();
    fvcs(&repo, &home).args(["push"]).assert().success();

    let mut state = 0x9E3779B9u32;
    let big: Vec<u8> = (0..1_500_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state >> 24) as u8
        })
        .collect();
    fs::write(repo.join("big.bin"), big).unwrap();
    fvcs(&repo, &home).args(["add", "."]).assert().success();
    fvcs(&repo, &home)
        .args(["commit", "-m", "too big"])
        .assert()
        .success();
    fvcs(&repo, &home)
        .args(["push"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("size limit"));
}

#[test]
fn registration_requires_pol_solution() {
    let tmp = TempDir::new().unwrap();
    let server = Server::start_with(&tmp.path().join("srv"), &["--pol-difficulty", "12"]);
    let token = fvcs::http_store::HttpStore::register(&server.base(), "human", "password123")
        .expect("a solved PoL should register fine");
    assert!(!token.is_empty());
}

#[test]
fn remote_delete_removes_repo_for_owner_only() {
    let tmp = TempDir::new().unwrap();
    let home_a = tmp.path().join("home-a");
    let home_b = tmp.path().join("home-b");
    fs::create_dir_all(&home_a).unwrap();
    fs::create_dir_all(&home_b).unwrap();
    let server = Server::start(&tmp.path().join("srv"));

    let alice = tmp.path().join("alice");
    init_avatar(&alice, &home_a, "Doomed", "soon gone");
    login(&home_a, &server, "alice");
    fvcs(&alice, &home_a)
        .args(["remote", "add", "origin", &server.repo_url("doomed-repo")])
        .assert()
        .success();
    fvcs(&alice, &home_a).args(["push"]).assert().success();

    let bob = tmp.path().join("bob");
    init_avatar(&bob, &home_b, "Bob", "not the owner");
    login(&home_b, &server, "bob");
    fvcs(&bob, &home_b)
        .args(["remote", "add", "origin", &server.repo_url("doomed-repo")])
        .assert()
        .success();
    fvcs(&bob, &home_b)
        .args(["remote", "delete", "origin", "--yes"])
        .assert()
        .failure();
    assert!(server.dir.join("doomed-repo").exists());

    fvcs(&alice, &home_a)
        .args(["remote", "delete", "origin"])
        .write_stdin("wrong-name\n")
        .assert()
        .failure();

    fvcs(&alice, &home_a)
        .args(["remote", "delete", "origin"])
        .write_stdin("doomed-repo\n")
        .assert()
        .success();
    assert!(!server.dir.join("doomed-repo").exists());

    fvcs(&alice, &home_a).args(["push"]).assert().success();
    assert!(server.dir.join("doomed-repo").exists());

    fvcs(&alice, &home_a)
        .args(["remote", "delete", "origin", "--yes"])
        .assert()
        .success();
    assert!(!server.dir.join("doomed-repo").exists());
}
