//! Minimal blocking HTTP client for api.music.yandex.net.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const BASE: &str = "https://api.music.yandex.net";
const ANDROID: &str = "YandexMusicAndroid/24023621";
pub const DESKTOP: &str = "YandexMusicDesktopAppWindows/5.13.2";

#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    token: String,
    #[cfg(test)]
    base: String,
}

impl Client {
    pub fn new(token: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .user_agent(concat!("yamusiq/", env!("CARGO_PKG_VERSION")))
            .build();
        Self {
            agent,
            token: token.trim().to_string(),
            #[cfg(test)]
            base: BASE.to_string(),
        }
    }

    #[cfg(test)]
    pub(super) fn for_test(base: String) -> Self {
        Self { base, ..Self::new("test-token") }
    }

    fn base(&self) -> &str {
        #[cfg(test)]
        { &self.base }
        #[cfg(not(test))]
        { BASE }
    }

    fn authed(&self, req: ureq::Request) -> ureq::Request {
        self.authed_as(req, ANDROID)
    }

    fn authed_as(&self, req: ureq::Request, client: &str) -> ureq::Request {
        req.set("Authorization", &format!("OAuth {}", self.token)).set("X-Yandex-Music-Client", client).set("Accept-Language", "ru")
    }

    /// GET `path` with query params and return the response's `result` field.
    pub fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        self.get_as(path, query, ANDROID)
    }

    /// Like [`get`](Self::get) but identifying as a specific client; some
    /// endpoints (e.g. `/get-file-info`) only answer the desktop app.
    pub fn get_as(&self, path: &str, query: &[(&str, &str)], client: &str) -> Result<Value> {
        let mut req = self.authed_as(self.agent.get(&format!("{}{path}", self.base())), client);
        for (k, v) in query {
            req = req.query(k, v);
        }
        result(req.call(), path)
    }

    /// POST a form to `path` and return the response's `result` field.
    pub fn post_form(&self, path: &str, form: &[(&str, &str)]) -> Result<Value> {
        let req = self.authed(self.agent.post(&format!("{}{path}", self.base())));
        result(req.send_form(form), path)
    }

    /// Fetch an absolute URL (download-info, lyrics files) as text.
    pub fn get_text(&self, url: &str, auth: bool) -> Result<String> {
        let mut req = self.agent.get(url);
        if auth {
            req = self.authed(req);
        }
        let resp = req.call().map_err(|e| http_error(e, url))?;
        resp.into_string().with_context(|| format!("reading {}", redact(url)))
    }
}

fn result(resp: Result<ureq::Response, ureq::Error>, path: &str) -> Result<Value> {
    let resp = resp.map_err(|e| http_error(e, path))?;
    let mut body: Value = resp.into_json().with_context(|| format!("{path}: invalid JSON"))?;
    match body.get_mut("result") {
        Some(v) => Ok(v.take()),
        None => Err(anyhow!("{path}: response without `result`")),
    }
}

/// Turns HTTP errors into readable messages; API errors carry a JSON body
/// like `{"error": {"name": "...", "message": "..."}}`.
fn http_error(e: ureq::Error, what: &str) -> anyhow::Error {
    match e {
        ureq::Error::Status(code, resp) => {
            let detail = resp
                .into_json::<Value>()
                .ok()
                .and_then(|v| {
                    let err = v.get("error")?;
                    let name = err.get("name").and_then(Value::as_str).unwrap_or("");
                    let msg = err.get("message").and_then(Value::as_str).unwrap_or("");
                    Some(format!("{name} {msg}").trim().to_string())
                })
                .unwrap_or_default();
            let hint = if code == 401 || code == 403 { " — проверьте токен" } else { "" };
            anyhow!("HTTP {code} {}{hint}: {}", detail, redact(what))
        }
        ureq::Error::Transport(t) => anyhow!("сеть: {t}"),
    }
}

/// Strip query strings (signatures, timestamps) from URLs in messages.
fn redact(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

pub fn is_status(err: &anyhow::Error, code: u16) -> bool {
    err.to_string().starts_with(&format!("HTTP {code}"))
}
