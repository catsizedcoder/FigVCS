use anyhow::{bail, Context, Result};
use reqwest::blocking::Client;
use reqwest::StatusCode;

use crate::commit::Commit;
use crate::remote::Store;

pub struct HttpStore {
    base: String,
    repo: String,
    token: Option<String>,
    client: Client,
}

impl HttpStore {
    pub fn new(url: &str, token: Option<&str>) -> Result<Self> {
        let trimmed = url.trim_end_matches('/');
        let split = trimmed
            .rfind('/')
            .context("server URLs look like https://host/repo-name")?;
        let (server, repo) = trimmed.split_at(split);
        let client = Client::builder()
            .user_agent(concat!("fvcs/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(HttpStore {
            base: format!("{server}/v1/repos/{}", &repo[1..]),
            repo: repo[1..].to_string(),
            token: token.map(|t| t.to_string()),
            client,
        })
    }

    pub fn login(server: &str, username: &str, password: &str) -> Result<String> {
        Self::auth_call(server, "login", username, password, serde_json::Value::Null)
    }

    pub fn register(server: &str, username: &str, password: &str) -> Result<String> {
        let client = Client::new();
        let pol_url = format!("{}/v1/pol", server.trim_end_matches('/'));
        let mut pol_field = serde_json::Value::Null;
        if let Ok(response) = client.get(&pol_url).send() {
            if response.status() == StatusCode::OK {
                let challenge: serde_json::Value = response.json()?;
                let text = challenge
                    .get("challenge")
                    .and_then(|c| c.as_str())
                    .unwrap_or_default()
                    .to_string();
                let difficulty = challenge
                    .get("difficulty")
                    .and_then(|d| d.as_u64())
                    .unwrap_or(0) as u32;
                if !text.is_empty() && difficulty > 0 {
                    eprintln!("solving a proof-of-life puzzle (anti-bot, one moment)...");
                    let nonce = crate::pol::solve(&text, difficulty);
                    pol_field = serde_json::json!({ "challenge": text, "nonce": nonce });
                }
            }
        }
        Self::auth_call(server, "register", username, password, pol_field)
    }

    fn auth_call(
        server: &str,
        endpoint: &str,
        username: &str,
        password: &str,
        extra: serde_json::Value,
    ) -> Result<String> {
        let client = Client::new();
        let mut body = serde_json::json!({ "username": username, "password": password });
        if !extra.is_null() {
            body["pol"] = extra;
        }
        let response = client
            .post(format!("{}/v1/{endpoint}", server.trim_end_matches('/')))
            .json(&body)
            .send()
            .with_context(|| format!("connecting to {server}"))?;
        match response.status() {
            StatusCode::OK | StatusCode::CREATED => {
                let body: serde_json::Value = response.json()?;
                body.get("token")
                    .and_then(|t| t.as_str())
                    .map(|t| t.to_string())
                    .context("server sent no token back")
            }
            StatusCode::UNAUTHORIZED => bail!("wrong username or password"),
            StatusCode::CONFLICT => bail!("that username is taken"),
            StatusCode::FORBIDDEN => bail!("this server has closed registration"),
            StatusCode::TOO_MANY_REQUESTS => {
                bail!("the server is rate-limiting you | slow down and try again later")
            }
            status => bail!("server error: {status}"),
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::blocking::RequestBuilder {
        let builder = self.client.request(method, format!("{}{path}", self.base));
        match &self.token {
            Some(token) => builder.bearer_auth(token),
            None => builder,
        }
    }

    fn get_bytes(&self, path: &str) -> Result<Option<Vec<u8>>> {
        let response = self
            .request(reqwest::Method::GET, path)
            .send()
            .with_context(|| format!("connecting to {}", self.base))?;
        match response.status() {
            StatusCode::OK => Ok(Some(response.bytes()?.to_vec())),
            StatusCode::NOT_FOUND => Ok(None),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                bail!("the server rejected your token | check `fvcs remote add`")
            }
            status => bail!("server error: {status}"),
        }
    }

    fn send(&self, method: reqwest::Method, path: &str, body: Vec<u8>) -> Result<()> {
        let response = self
            .request(method, path)
            .body(body)
            .send()
            .with_context(|| format!("connecting to {}", self.base))?;
        match response.status() {
            StatusCode::OK | StatusCode::CREATED | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                bail!("the server rejected your token | check `fvcs remote add`")
            }
            StatusCode::CONFLICT => {
                bail!("the server refused the push | someone else pushed first, run `fvcs pull`")
            }
            StatusCode::TOO_MANY_REQUESTS => {
                bail!("the server is rate-limiting you | slow down and try again later")
            }
            StatusCode::PAYLOAD_TOO_LARGE => {
                bail!("the repo is over the server's size limit")
            }
            status => {
                let text = response.text().unwrap_or_default();
                bail!("server error: {status} {text}")
            }
        }
    }
}

impl Store for HttpStore {
    fn head_branch(&self) -> Result<Option<String>> {
        match self.get_bytes("/head")? {
            Some(bytes) => Ok(Some(String::from_utf8(bytes)?.trim().to_string())),
            None => Ok(None),
        }
    }

    fn branch_commit(&self, branch: &str) -> Result<Option<String>> {
        match self.get_bytes(&format!("/refs/heads/{branch}"))? {
            Some(bytes) => Ok(Some(String::from_utf8(bytes)?.trim().to_string())),
            None => Ok(None),
        }
    }

    fn update_branch(&self, branch: &str, hash: &str) -> Result<()> {
        self.send(
            reqwest::Method::PUT,
            &format!("/refs/heads/{branch}"),
            hash.as_bytes().to_vec(),
        )
    }

    fn has_commit(&self, hash: &str) -> Result<bool> {
        Ok(self.get_bytes(&format!("/commits/{hash}"))?.is_some())
    }

    fn read_commit(&self, hash: &str) -> Result<Commit> {
        let bytes = self
            .get_bytes(&format!("/commits/{hash}"))?
            .with_context(|| format!("commit {hash} missing on the server"))?;
        Commit::from_json(&bytes)
    }

    fn write_commit(&self, commit: &Commit) -> Result<String> {
        let json = commit.to_json()?;
        let hash = crate::object::hash_bytes(&json);
        self.send(reqwest::Method::POST, &format!("/commits/{hash}"), json)?;
        Ok(hash)
    }

    fn has_object(&self, hash: &str) -> Result<bool> {
        Ok(self.get_bytes(&format!("/objects/{hash}"))?.is_some())
    }

    fn read_object_wire(&self, hash: &str) -> Result<Vec<u8>> {
        self.get_bytes(&format!("/objects/{hash}"))?
            .with_context(|| format!("object {hash} missing on the server"))
    }

    fn write_object_wire(&self, hash: &str, compressed: &[u8]) -> Result<()> {
        self.send(
            reqwest::Method::POST,
            &format!("/objects/{hash}"),
            compressed.to_vec(),
        )
    }

    fn write_readme(&self, content: &str) -> Result<()> {
        self.send(
            reqwest::Method::POST,
            "/readme",
            content.as_bytes().to_vec(),
        )
    }

    fn repo_name(&self) -> Option<String> {
        Some(self.repo.clone())
    }

    fn delete_repo(&self) -> Result<()> {
        let response = self.request(reqwest::Method::DELETE, "").send()?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                bail!("only the repo owner can delete it")
            }
            StatusCode::NOT_FOUND => bail!("that repo does not exist on the server"),
            status => bail!("server error: {status}"),
        }
    }

    fn set_visibility(&self, visibility: &str) -> Result<()> {
        self.send(
            reqwest::Method::PUT,
            "/visibility",
            visibility.as_bytes().to_vec(),
        )
    }

    fn share(&self, username: &str, remove: bool) -> Result<()> {
        let body = serde_json::json!({ "username": username, "remove": remove })
            .to_string()
            .into_bytes();
        let response = self
            .request(reqwest::Method::PUT, "/share")
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()?;
        match response.status() {
            StatusCode::NO_CONTENT => Ok(()),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => {
                bail!("only the repo owner can change sharing")
            }
            status => bail!("server error: {status}"),
        }
    }
}
