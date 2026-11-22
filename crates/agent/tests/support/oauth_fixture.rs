use anyhow::{Result,
             ensure};
use gpui_agent::{McpOAuthCommand,
                 McpOAuthState,
                 McpServer};
use serde_json::json;
use std::{io::{Read,
               Write},
          net::TcpListener,
          sync::{Arc,
                 atomic::{AtomicBool,
                          Ordering}},
          time::Duration};
pub struct OAuthFixture {
    pub origin: String,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<Result<()>>>,
}
impl OAuthFixture {
    pub fn new(mode: &'static str) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let origin = format!("http://{}", listener.local_addr()?);
        let url = origin.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let worker = std::thread::spawn(move || -> Result<()> {
            while !stopped.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    | Ok(v) => v,
                    | Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    },
                    | Err(e) => return Err(e.into()),
                };
                stream.set_nonblocking(false)?;
                stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                let mut bytes = Vec::new();
                let mut buf = [0; 4096];
                let end = loop {
                    let n = stream.read(&mut buf)?;
                    ensure!(n > 0);
                    bytes.extend_from_slice(&buf[.. n]);
                    if let Some(i) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let headers = String::from_utf8(bytes[.. end].to_vec())?;
                let length = headers
                    .lines()
                    .find_map(|s| {
                        s.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while bytes.len() < end + length {
                    let n = stream.read(&mut buf)?;
                    ensure!(n > 0);
                    bytes.extend_from_slice(&buf[.. n]);
                }
                let path = headers.split_whitespace().nth(1).unwrap();
                let (code, body) = match path {
                    | "/mcp" => (401, json!({})),
                    | "/metadata" => (
                        200,
                        json!({"resource":if mode=="wrong-resource"{format!("{url}/other")}else{format!("{url}/mcp")},"authorization_servers":[url]}),
                    ),
                    | "/.well-known/oauth-authorization-server" => (
                        200,
                        json!({"issuer":if mode=="wrong-issuer"{format!("{url}/wrong")}else{url.clone()},"authorization_endpoint":if mode=="duplicate-query"{format!("{url}/authorize?state=unsafe")}else{format!("{url}/authorize")},"token_endpoint":format!("{url}/token"),"registration_endpoint":format!("{url}/register"),"authorization_response_iss_parameter_supported":mode=="require-issuer","code_challenge_methods_supported":if mode=="no-pkce"{json!(["plain"])}else{json!(["S256"])}}),
                    ),
                    | "/register" => (200, json!({"client_id":"fixture-public","token_endpoint_auth_method":"none"})),
                    | "/token" if mode == "token-error" => (400, json!({"error_description":"SECRET_MUST_NOT_LEAK"})),
                    | "/token" => {
                        let query = reqwest::Url::parse(&format!("http://fixture.invalid/?{}", std::str::from_utf8(&bytes[end ..])?))?;
                        let form: std::collections::HashMap<_, _> = query.query_pairs().into_owned().collect();
                        ensure!(form["client_id"] == "fixture-public" && form["resource"] == format!("{url}/mcp"));
                        ensure!(form["grant_type"] == "authorization_code" && form["code"] == "fixture-code" && form["code_verifier"].len() == 43);
                        let mut token =
                            json!({"access_token":"fixture-oauth-access","refresh_token":"fixture-oauth-refresh","token_type":"Bearer","expires_in":3600});
                        if mode == "no-expiry" {
                            token.as_object_mut().unwrap().remove("expires_in");
                        }
                        if mode == "no-refresh" {
                            token.as_object_mut().unwrap().remove("refresh_token");
                            token["expires_in"] = json!(20);
                        }
                        (200, token)
                    },
                    | _ => (404, json!({})),
                };
                let extra = if path == "/mcp" {
                    format!("WWW-Authenticate: Bearer resource_metadata=\"{url}/metadata\", scope=\"tools:read\"\r\n")
                } else {
                    String::new()
                };
                let body = body.to_string();
                write!(
                    stream,
                    "HTTP/1.1 {code} Result\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )?;
            }
            Ok(())
        });
        Ok(Self {
            origin,
            stop,
            worker: Some(worker),
        })
    }

    pub fn server(&self) -> McpServer { serde_json::from_value(json!({"name":"remote","url":format!("{}/mcp",self.origin),"oauth":{}})).unwrap() }

    pub fn login(&self, home: &std::path::Path, oauth: &gpui_agent::McpOAuth) -> Result<()> {
        let status = oauth.execute(home, &self.server(), McpOAuthCommand::Login)?;
        let callback = callback(status.authorization_url.as_deref().unwrap(), "fixture-code")?;
        ensure!(reqwest::blocking::get(callback)?.status().is_success());
        ensure!(oauth.execute(home, &self.server(), McpOAuthCommand::Status)?.state == McpOAuthState::Connected);
        Ok(())
    }
}
impl Drop for OAuthFixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap().expect("OAuth fixture failed");
        }
    }
}
pub fn callback(authorization: &str, code: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(authorization)?;
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    let mut callback = reqwest::Url::parse(&query["redirect_uri"])?;
    callback.query_pairs_mut().append_pair("state", &query["state"]).append_pair("code", code);
    Ok(callback)
}
