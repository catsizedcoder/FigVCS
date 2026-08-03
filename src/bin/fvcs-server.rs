use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path as AxPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use clap::Parser;
use fvcs::commit::Commit;
use fvcs::object;
use fvcs::refs;
use fvcs::remote::{self, Store};
use fvcs::repo::Repository;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Parser)]
#[command(name = "fvcs-server", version, about = "Serve FigVCS repos over HTTP")]
struct Args {
    #[arg(long, default_value = ".")]
    dir: PathBuf,
    #[arg(long, default_value = "8080")]
    port: u16,
    #[arg(long, help = "Disable open account registration")]
    closed: bool,
    #[arg(
        long,
        help = "Trust X-Forwarded-For for rate limits (only enable behind a proxy like Cloudflare/nginx)"
    )]
    use_proxy_headers: bool,
    #[arg(
        long,
        default_value = "18",
        help = "Proof-of-life puzzle difficulty in bits (0 disables)"
    )]
    pol_difficulty: u32,
    #[arg(
        long,
        default_value = "8",
        help = "Extra PoL bits added per recent registration from the same IP"
    )]
    pol_adaptive_max: u32,
    #[arg(long, default_value = "5", help = "Max registrations per IP per hour")]
    rate_register: usize,
    #[arg(long, default_value = "20", help = "Max logins per IP per hour")]
    rate_login: usize,
    #[arg(long, default_value = "600", help = "Max pushes per IP per hour")]
    rate_push: usize,
    #[arg(
        long,
        default_value = "200",
        help = "Max pushes per account per day (0 = unlimited)"
    )]
    max_pushes_per_day: i64,
    #[arg(long, help = "Run garbage collection once and exit")]
    gc: bool,
    #[arg(
        long,
        default_value = "0",
        help = "Run garbage collection every N hours while serving (0 = off)"
    )]
    gc_interval_hours: u64,
    #[arg(
        long,
        default_value = "1",
        help = "Objects younger than this many hours are never collected"
    )]
    gc_grace_hours: u64,
    #[arg(
        long,
        default_value = "0",
        help = "Max size of one repo in MiB, commits plus referenced objects (0 = unlimited)"
    )]
    max_repo_size_mb: u64,
}

struct AppState {
    dir: PathBuf,
    objects: PathBuf,
    db: Mutex<Connection>,
    registration_open: bool,
    use_proxy_headers: bool,
    pol_difficulty: u32,
    pol_adaptive_max: u32,
    pol_secret: String,
    used_pol: Mutex<HashMap<String, Instant>>,
    rate: Mutex<HashMap<String, VecDeque<Instant>>>,
    rate_register: usize,
    rate_login: usize,
    rate_push: usize,
    max_pushes_per_day: i64,
    max_repo_size_bytes: u64,
}

#[derive(Deserialize)]
struct Credentials {
    username: String,
    password: String,
    #[serde(default)]
    pol: Option<PolAnswer>,
}

#[derive(Deserialize)]
struct PolAnswer {
    challenge: String,
    nonce: u64,
}

#[derive(Serialize)]
struct PolChallenge {
    challenge: String,
    difficulty: u32,
}

#[derive(Serialize)]
struct TokenReply {
    token: String,
}

#[derive(Deserialize)]
struct ShareRequest {
    username: String,
    #[serde(default)]
    remove: bool,
}

fn hex_hash(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("system randomness unavailable");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_password(salt: &str, password: &str) -> String {
    hex_hash(format!("{salt}:{password}").as_bytes())
}

fn open_db(dir: &Path) -> Result<Connection> {
    let db = Connection::open(dir.join("server.db")).context("opening server.db")?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS users (
            username TEXT PRIMARY KEY,
            salt TEXT NOT NULL,
            pass_hash TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tokens (
            token TEXT PRIMARY KEY,
            username TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS repos (
            name TEXT PRIMARY KEY,
            owner TEXT NOT NULL,
            visibility TEXT NOT NULL DEFAULT 'private'
        );
        CREATE TABLE IF NOT EXISTS collaborators (
            repo TEXT NOT NULL,
            username TEXT NOT NULL,
            PRIMARY KEY (repo, username)
        );
        CREATE TABLE IF NOT EXISTS push_counts (
            username TEXT NOT NULL,
            day TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (username, day)
        );",
    )?;
    Ok(db)
}

fn auth_user(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))?;
    let db = state.db.lock().ok()?;
    db.query_row(
        "SELECT username FROM tokens WHERE token = ?1",
        [token],
        |row| row.get(0),
    )
    .ok()
}

fn rate_limited(state: &AppState, bucket: &str, ip: &str, max: usize) -> bool {
    if max == 0 {
        return false;
    }
    let Ok(mut map) = state.rate.lock() else {
        return false;
    };
    let now = Instant::now();
    let window = Duration::from_secs(3600);
    let hits = map.entry(format!("{bucket}:{ip}")).or_default();
    while hits
        .front()
        .is_some_and(|t| now.duration_since(*t) > window)
    {
        hits.pop_front();
    }
    if hits.len() >= max {
        return true;
    }
    hits.push_back(now);
    false
}

fn push_quota_left(state: &AppState, username: &str) -> bool {
    if state.max_pushes_per_day == 0 {
        return true;
    }
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let Ok(db) = state.db.lock() else {
        return false;
    };
    let count: i64 = db
        .query_row(
            "SELECT count FROM push_counts WHERE username = ?1 AND day = ?2",
            [username, &day],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if count >= state.max_pushes_per_day {
        return false;
    }
    db.execute(
        "INSERT INTO push_counts (username, day, count) VALUES (?1, ?2, 1)
         ON CONFLICT (username, day) DO UPDATE SET count = count + 1",
        [username, &day],
    )
    .is_ok()
}

fn repo_meta(state: &AppState, repo: &str) -> Option<(String, String)> {
    let db = state.db.lock().ok()?;
    db.query_row(
        "SELECT owner, visibility FROM repos WHERE name = ?1",
        [repo],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .ok()
}

fn is_collaborator(state: &AppState, repo: &str, username: &str) -> bool {
    let Ok(db) = state.db.lock() else {
        return false;
    };
    db.query_row(
        "SELECT 1 FROM collaborators WHERE repo = ?1 AND username = ?2",
        [repo, username],
        |_| Ok(()),
    )
    .is_ok()
}

fn valid_repo_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

fn open_repo_dir(state: &AppState, name: &str) -> Result<Repository, StatusCode> {
    if !valid_repo_name(name) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let path = state.dir.join(name);
    if !path.join(".fvcs").is_dir() {
        return Err(StatusCode::NOT_FOUND);
    }
    Repository::with_shared_objects(path, state.objects.clone())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

fn guard_read(state: &AppState, headers: &HeaderMap, name: &str) -> Result<Repository, StatusCode> {
    let repo = open_repo_dir(state, name)?;
    if let Some((owner, visibility)) = repo_meta(state, name) {
        if visibility == "private" {
            let user = auth_user(state, headers);
            let allowed = user
                .as_ref()
                .is_some_and(|u| u == &owner || is_collaborator(state, name, u));
            if !allowed {
                return Err(StatusCode::NOT_FOUND);
            }
        }
    }
    Ok(repo)
}

fn guard_write(
    state: &AppState,
    headers: &HeaderMap,
    name: &str,
) -> Result<Repository, StatusCode> {
    if !valid_repo_name(name) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let Some(user) = auth_user(state, headers) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    match repo_meta(state, name) {
        Some((owner, _)) => {
            if user != owner && !is_collaborator(state, name, &user) {
                return Err(StatusCode::FORBIDDEN);
            }
            open_repo_dir(state, name)
        }
        None => {
            let repo = Repository::with_shared_objects(state.dir.join(name), state.objects.clone())
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            let db = state
                .db
                .lock()
                .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            db.execute(
                "INSERT INTO repos (name, owner, visibility) VALUES (?1, ?2, 'private')",
                [name, &user],
            )
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
            Ok(repo)
        }
    }
}

fn client_ip(state: &AppState, addr: &SocketAddr, headers: &HeaderMap) -> String {
    if state.use_proxy_headers {
        if let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
        {
            let ip = forwarded.trim();
            if !ip.is_empty() {
                return ip.to_string();
            }
        }
    }
    addr.ip().to_string()
}

fn pol_sign(secret: &str, payload: &str) -> String {
    hex_hash(format!("{secret}:{payload}").as_bytes())
}

fn pol_difficulty_for(state: &AppState, ip: &str) -> u32 {
    let recent = state
        .rate
        .lock()
        .ok()
        .and_then(|map| map.get(&format!("register:{ip}")).map(|q| q.len()))
        .unwrap_or(0) as u32;
    state.pol_difficulty + recent.min(state.pol_adaptive_max)
}

async fn get_pol(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if state.pol_difficulty == 0 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let ip = client_ip(&state, &addr, &headers);
    let difficulty = pol_difficulty_for(&state, &ip);
    let expiry = chrono::Utc::now().timestamp() + 600;
    let payload = format!("{}.{expiry}.{difficulty}", random_hex(16));
    let challenge = format!("{payload}.{}", pol_sign(&state.pol_secret, &payload));
    Json(PolChallenge {
        challenge,
        difficulty,
    })
    .into_response()
}

fn check_pol(state: &AppState, answer: &Option<PolAnswer>) -> bool {
    if state.pol_difficulty == 0 {
        return true;
    }
    let Some(answer) = answer else {
        return false;
    };
    let parts: Vec<&str> = answer.challenge.split('.').collect();
    let [rand, expiry, difficulty, signature] = parts.as_slice() else {
        return false;
    };
    let payload = format!("{rand}.{expiry}.{difficulty}");
    if pol_sign(&state.pol_secret, &payload) != *signature {
        return false;
    }
    let Ok(mut used) = state.used_pol.lock() else {
        return false;
    };
    used.retain(|_, at| at.elapsed() < Duration::from_secs(600));
    if used.contains_key(&payload) {
        return false;
    }
    let (Ok(expiry), Ok(difficulty)) = (expiry.parse::<i64>(), difficulty.parse::<u32>()) else {
        return false;
    };
    if difficulty < state.pol_difficulty {
        return false;
    }
    if chrono::Utc::now().timestamp() > expiry {
        return false;
    }
    if !fvcs::pol::verify(&answer.challenge, answer.nonce, difficulty) {
        return false;
    }
    used.insert(payload, Instant::now());
    true
}

async fn register(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(creds): Json<Credentials>,
) -> impl IntoResponse {
    if !state.registration_open {
        return StatusCode::FORBIDDEN.into_response();
    }
    if rate_limited(
        &state,
        "register",
        &client_ip(&state, &addr, &headers),
        state.rate_register,
    ) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    if !check_pol(&state, &creds.pol) {
        return (
            StatusCode::BAD_REQUEST,
            "proof-of-life check failed or missing",
        )
            .into_response();
    }
    let valid = creds.username.len() >= 2
        && creds
            .username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && creds.password.len() >= 6;
    if !valid {
        return (
            StatusCode::BAD_REQUEST,
            "username: 2+ chars [a-z0-9-_], password: 6+ chars",
        )
            .into_response();
    }
    let salt = random_hex(16);
    let pass_hash = hash_password(&salt, &creds.password);
    let token = random_hex(32);
    let result = state
        .db
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        .and_then(|db| {
            db.execute(
                "INSERT INTO users (username, salt, pass_hash) VALUES (?1, ?2, ?3)",
                [&creds.username, &salt, &pass_hash],
            )
            .map_err(|_| StatusCode::CONFLICT)?;
            db.execute(
                "INSERT INTO tokens (token, username) VALUES (?1, ?2)",
                [&token, &creds.username],
            )
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
        });
    match result {
        Ok(_) => (StatusCode::CREATED, Json(TokenReply { token })).into_response(),
        Err(status) => status.into_response(),
    }
}

async fn login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(creds): Json<Credentials>,
) -> impl IntoResponse {
    if rate_limited(
        &state,
        "login",
        &client_ip(&state, &addr, &headers),
        state.rate_login,
    ) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Ok(db) = state.db.lock() else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let row = db.query_row(
        "SELECT salt, pass_hash FROM users WHERE username = ?1",
        [&creds.username],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    );
    let Ok((salt, pass_hash)) = row else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if hash_password(&salt, &creds.password) != pass_hash {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let token = random_hex(32);
    if db
        .execute(
            "INSERT INTO tokens (token, username) VALUES (?1, ?2)",
            [&token, &creds.username],
        )
        .is_err()
    {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    Json(TokenReply { token }).into_response()
}

async fn set_visibility(
    State(state): State<Arc<AppState>>,
    AxPath(repo): AxPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let visibility = String::from_utf8_lossy(&body).trim().to_string();
    if visibility != "public" && visibility != "private" {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(user) = auth_user(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match repo_meta(&state, &repo) {
        Some((owner, _)) if owner == user => {
            let Ok(db) = state.db.lock() else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            match db.execute(
                "UPDATE repos SET visibility = ?1 WHERE name = ?2",
                [&visibility, &repo],
            ) {
                Ok(_) => StatusCode::NO_CONTENT.into_response(),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Some(_) => StatusCode::FORBIDDEN.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn share_repo(
    State(state): State<Arc<AppState>>,
    AxPath(repo): AxPath<String>,
    headers: HeaderMap,
    Json(request): Json<ShareRequest>,
) -> impl IntoResponse {
    let Some(user) = auth_user(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match repo_meta(&state, &repo) {
        Some((owner, _)) if owner == user => {
            let Ok(db) = state.db.lock() else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            let result = if request.remove {
                db.execute(
                    "DELETE FROM collaborators WHERE repo = ?1 AND username = ?2",
                    [&repo, &request.username],
                )
            } else {
                db.execute(
                    "INSERT OR IGNORE INTO collaborators (repo, username) VALUES (?1, ?2)",
                    [&repo, &request.username],
                )
            };
            match result {
                Ok(_) => StatusCode::NO_CONTENT.into_response(),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Some(_) => StatusCode::FORBIDDEN.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn delete_repo(
    State(state): State<Arc<AppState>>,
    AxPath(repo): AxPath<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !valid_repo_name(&repo) {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(user) = auth_user(&state, &headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match repo_meta(&state, &repo) {
        Some((owner, _)) if owner == user => {
            let path = state.dir.join(&repo);
            let Ok(db) = state.db.lock() else {
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            };
            let deleted = db
                .execute("DELETE FROM repos WHERE name = ?1", [&repo])
                .and_then(|_| db.execute("DELETE FROM collaborators WHERE repo = ?1", [&repo]));
            drop(db);
            match deleted {
                Ok(_) => {
                    if path.exists() {
                        let _ = std::fs::remove_dir_all(path);
                    }
                    StatusCode::NO_CONTENT.into_response()
                }
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Some(_) => StatusCode::FORBIDDEN.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_head(
    State(state): State<Arc<AppState>>,
    AxPath(repo): AxPath<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match guard_read(&state, &headers, &repo) {
        Ok(r) => match Store::head_branch(&r) {
            Ok(Some(branch)) => (StatusCode::OK, branch).into_response(),
            _ => StatusCode::NOT_FOUND.into_response(),
        },
        Err(status) => status.into_response(),
    }
}

async fn get_branch(
    State(state): State<Arc<AppState>>,
    AxPath((repo, branch)): AxPath<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match guard_read(&state, &headers, &repo) {
        Ok(r) => match refs::branch_commit(&r, &branch) {
            Ok(Some(hash)) => (StatusCode::OK, hash).into_response(),
            _ => StatusCode::NOT_FOUND.into_response(),
        },
        Err(status) => status.into_response(),
    }
}

async fn put_branch(
    State(state): State<Arc<AppState>>,
    AxPath((repo, branch)): AxPath<(String, String)>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    if rate_limited(
        &state,
        "push",
        &client_ip(&state, &addr, &headers),
        state.rate_push,
    ) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let hash = String::from_utf8_lossy(&body).trim().to_string();
    match guard_write(&state, &headers, &repo) {
        Ok(r) => {
            let user = auth_user(&state, &headers).unwrap_or_default();
            if !push_quota_left(&state, &user) {
                return StatusCode::TOO_MANY_REQUESTS.into_response();
            }
            let conflict = refs::branch_commit(&r, &branch)
                .ok()
                .flatten()
                .is_some_and(|current| {
                    current != hash && !remote::is_ancestor(&r, &current, &hash).unwrap_or(false)
                });
            if conflict {
                return StatusCode::CONFLICT.into_response();
            }
            if state.max_repo_size_bytes > 0
                && fvcs::gc::repo_size(&r, &hash).unwrap_or(0) > state.max_repo_size_bytes
            {
                return StatusCode::PAYLOAD_TOO_LARGE.into_response();
            }
            match refs::update_branch(&r, &branch, &hash) {
                Ok(()) => StatusCode::NO_CONTENT.into_response(),
                Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
            }
        }
        Err(status) => status.into_response(),
    }
}

async fn get_commit(
    State(state): State<Arc<AppState>>,
    AxPath((repo, hash)): AxPath<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match guard_read(&state, &headers, &repo) {
        Ok(r) => match std::fs::read(r.commits().join(format!("{hash}.json"))) {
            Ok(bytes) => (StatusCode::OK, bytes).into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        },
        Err(status) => status.into_response(),
    }
}

async fn post_commit(
    State(state): State<Arc<AppState>>,
    AxPath((repo, hash)): AxPath<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let r = match guard_write(&state, &headers, &repo) {
        Ok(r) => r,
        Err(status) => return status.into_response(),
    };
    let result = (|| -> Result<()> {
        let commit = Commit::from_json(&body)?;
        anyhow::ensure!(
            object::hash_bytes(&commit.to_json()?) == hash,
            "hash mismatch"
        );
        commit.store(&r)?;
        Ok(())
    })();
    match result {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn get_object(
    State(state): State<Arc<AppState>>,
    AxPath((repo, hash)): AxPath<(String, String)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match guard_read(&state, &headers, &repo) {
        Ok(r) => match object::read_compressed(&r, &hash) {
            Ok(bytes) => (StatusCode::OK, bytes).into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        },
        Err(status) => status.into_response(),
    }
}

async fn post_object(
    State(state): State<Arc<AppState>>,
    AxPath((repo, hash)): AxPath<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    let r = match guard_write(&state, &headers, &repo) {
        Ok(r) => r,
        Err(status) => return status.into_response(),
    };
    let result = (|| -> Result<()> {
        let raw = object::decompress(&body)?;
        anyhow::ensure!(object::hash_bytes(&raw) == hash, "hash mismatch");
        object::write_compressed(&r, &hash, &body)?;
        Ok(())
    })();
    match result {
        Ok(()) => StatusCode::CREATED.into_response(),
        Err(_) => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn post_readme(
    State(state): State<Arc<AppState>>,
    AxPath(repo): AxPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    match guard_write(&state, &headers, &repo) {
        Ok(r) => match r.write_readme(&String::from_utf8_lossy(&body)) {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        },
        Err(status) => status.into_response(),
    }
}

async fn get_registry(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match std::fs::read(state.dir.join("registry.json")) {
        Ok(bytes) => (StatusCode::OK, bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn load_pol_secret(dir: &Path) -> String {
    let path = dir.join(".pol-secret");
    if let Ok(secret) = std::fs::read_to_string(&path) {
        let secret = secret.trim().to_string();
        if !secret.is_empty() {
            return secret;
        }
    }
    let secret = random_hex(32);
    let _ = std::fs::write(&path, &secret);
    secret
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    std::fs::create_dir_all(&args.dir).context("creating repos dir")?;

    let grace = Duration::from_secs(args.gc_grace_hours * 3600);
    if args.gc {
        let stats = fvcs::gc::collect(&args.dir, grace)?;
        println!(
            "gc done | {} repos scanned, {} orphaned commits removed, {} objects removed ({} bytes freed), {} objects kept",
            stats.repos_scanned,
            stats.commits_removed,
            stats.objects_removed,
            stats.bytes_freed,
            stats.objects_kept
        );
        return Ok(());
    }

    let state = Arc::new(AppState {
        dir: args.dir.canonicalize()?,
        objects: args.dir.join("objects-pool"),
        db: Mutex::new(open_db(&args.dir)?),
        registration_open: !args.closed,
        use_proxy_headers: args.use_proxy_headers,
        pol_difficulty: args.pol_difficulty,
        pol_adaptive_max: args.pol_adaptive_max,
        pol_secret: load_pol_secret(&args.dir),
        used_pol: Mutex::new(HashMap::new()),
        rate: Mutex::new(HashMap::new()),
        rate_register: args.rate_register,
        rate_login: args.rate_login,
        rate_push: args.rate_push,
        max_pushes_per_day: args.max_pushes_per_day,
        max_repo_size_bytes: args.max_repo_size_mb * 1024 * 1024,
    });
    let app = Router::new()
        .route("/v1/register", post(register))
        .route("/v1/login", post(login))
        .route("/v1/pol", get(get_pol))
        .route("/v1/repos/{repo}/head", get(get_head))
        .route("/v1/repos/{repo}/visibility", put(set_visibility))
        .route("/v1/repos/{repo}/share", put(share_repo))
        .route("/v1/repos/{repo}", delete(delete_repo))
        .route(
            "/v1/repos/{repo}/refs/heads/{branch}",
            get(get_branch).put(put_branch),
        )
        .route(
            "/v1/repos/{repo}/commits/{hash}",
            get(get_commit).post(post_commit),
        )
        .route(
            "/v1/repos/{repo}/objects/{hash}",
            get(get_object).post(post_object),
        )
        .route("/v1/repos/{repo}/readme", post(post_readme))
        .route("/registry.json", get(get_registry))
        .with_state(state);
    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    println!("fvcs-server listening on http://{addr}");

    if args.gc_interval_hours > 0 {
        let dir = args.dir.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(args.gc_interval_hours * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                match fvcs::gc::collect(&dir, grace) {
                    Ok(stats) => println!(
                        "gc | {} commits removed, {} objects removed ({} bytes freed)",
                        stats.commits_removed, stats.objects_removed, stats.bytes_freed
                    ),
                    Err(err) => eprintln!("gc failed: {err:#}"),
                }
            }
        });
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}
