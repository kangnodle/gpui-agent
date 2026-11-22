//! OAuth network, loopback callback and profile credential effects.
use anyhow::{Context,
             Result,
             bail,
             ensure};
use base64::{Engine,
             engine::general_purpose::URL_SAFE_NO_PAD};
use gpui_agent_domain::{McpOAuthCommand as Command,
                        McpOAuthState as State,
                        McpOAuthStatus as Status,
                        McpServer,
                        mcp_oauth_endpoint,
                        mcp_oauth_loopback as loopback};
use serde::{Deserialize,
            Serialize};
use serde_json::{Value,
                 json};
use sha2::{Digest,
           Sha256};
use std::{collections::{BTreeMap,
                        HashMap},
          io::{Read,
               Write},
          net::{TcpListener,
                TcpStream,
                ToSocketAddrs},
          path::{Path,
                 PathBuf},
          sync::{Arc,
                 Mutex,
                 atomic::{AtomicBool,
                          Ordering}},
          time::{Duration,
                 Instant,
                 SystemTime,
                 UNIX_EPOCH}};
use url::Url;
fn endpoint(value: &str, resource: &str) -> Result<Url> { mcp_oauth_endpoint(value, resource).map_err(anyhow::Error::msg) }

#[derive(Default)]
pub struct McpOAuth {
    attempts: Mutex<HashMap<PathBuf, Arc<Attempt>>>,
}
struct Attempt {
    cancel: AtomicBool,
    status: Mutex<Status>,
    deadline: Instant,
}
impl Drop for McpOAuth {
    fn drop(&mut self) {
        if let Ok(attempts) = self.attempts.get_mut() {
            for attempt in attempts.values() {
                attempt.cancel.store(true, Ordering::SeqCst);
            }
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Credential {
    identity: String,
    resource: String,
    client_id: String,
    token_endpoint: String,
    access_token: String,
    refresh_token: Option<String>,
    expires_at: Option<u64>,
}
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }
fn random() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes).map_err(|_| anyhow::anyhow!("OAuth randomness unavailable"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
/// Records a terminal login failure unless the user already cancelled. Call it
/// before replying to the browser so a client that sees the reply never reads
/// a stale `Pending` status.
fn fail(attempt: &Attempt) {
    if !attempt.cancel.load(Ordering::SeqCst)
        && let Ok(mut s) = attempt.status.lock()
    {
        *s = status(if Instant::now() >= attempt.deadline { State::Expired } else { State::Failed });
        s.message = Some("OAuth login failed or was declined. Retry from MCP settings.".into());
    }
}
fn status(state: State) -> Status {
    Status {
        state,
        authorization_url: None,
        expires_at: None,
        message: None,
    }
}
fn identity(server: &McpServer) -> Result<String> { Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(server)?))) }
fn path(home: &Path, server: &McpServer) -> PathBuf { home.join("mcp-oauth").join(format!("{:x}.json", Sha256::digest(server.name().as_bytes()))) }
fn lock(home: &Path) -> Result<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(home.join(".mcp-oauth.lock"))?;
    file.lock()?;
    Ok(file)
}
fn read(home: &Path, server: &McpServer) -> Result<Option<Credential>> {
    let file = match std::fs::File::open(path(home, server)) {
        | Ok(f) => f,
        | Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        | Err(e) => return Err(e.into()),
    };
    let credential: Credential = serde_json::from_reader(file.take(65537)).map_err(|_| anyhow::anyhow!("Invalid MCP OAuth credential file; log in again"))?;
    Ok((credential.identity == identity(server)?).then_some(credential))
}
fn save(home: &Path, server: &McpServer, credential: &Credential) -> Result<()> {
    let target = path(home, server);
    let directory = target.parent().context("Missing credential directory")?;
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut file, credential)?;
    file.as_file().sync_all()?;
    file.persist(target).map_err(|e| e.error)?;
    Ok(())
}
fn request(method: reqwest::Method, target: &str, resource: &str) -> Result<reqwest::blocking::RequestBuilder> {
    let url = endpoint(target, resource)?;
    let mut builder = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10));
    let resource_url = Url::parse(resource)?;
    if !loopback(&resource_url) && url.origin() != resource_url.origin() {
        let host = url.host_str().context("Missing OAuth host")?;
        let addresses: Vec<_> = (host, url.port_or_known_default().context("Missing OAuth port")?)
            .to_socket_addrs()
            .map_err(|_| anyhow::anyhow!("OAuth endpoint DNS lookup failed"))?
            .collect();
        ensure!(
            !addresses.is_empty() && addresses.iter().all(|address| public_ip(address.ip())),
            "OAuth discovery cannot access a private network"
        );
        builder = builder.resolve_to_addrs(host, &addresses);
    }
    Ok(builder.build()?.request(method, url))
}
fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        | std::net::IpAddr::V4(ip) =>
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_unspecified()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && !ip.is_multicast()
                && ip.octets()[0] != 0
                && ip.octets()[0] < 240
                && !(ip.octets()[0] == 100 && (64 ..= 127).contains(&ip.octets()[1])),
        | std::net::IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(|ip| public_ip(ip.into()))
            .unwrap_or(!ip.is_loopback() && !ip.is_unspecified() && !ip.is_unique_local() && !ip.is_unicast_link_local() && !ip.is_multicast()),
    }
}
fn response_json(response: reqwest::blocking::Response) -> Result<Value> {
    ensure!(
        response.status().is_success(),
        "OAuth endpoint rejected the request ({})",
        response.status().as_u16()
    );
    let mut bytes = Vec::new();
    response
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow::anyhow!("Cannot read OAuth response"))?;
    ensure!(bytes.len() <= 65536, "OAuth response is too large");
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid OAuth JSON response"))
}
fn get(target: &str, resource: &str) -> Result<Value> {
    let response = request(reqwest::Method::GET, target, resource)?
        .send()
        .map_err(|_| anyhow::anyhow!("OAuth discovery request failed"))?;
    response_json(response)
}
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 4096)
        .with_context(|| format!("OAuth metadata is missing {name}"))
}
fn credential(
    server: &McpServer,
    resource: &str,
    client_id: &str,
    token_endpoint: &str,
    form: &[(&str, &str)],
    previous_refresh: Option<String>,
) -> Result<Credential> {
    let response = request(reqwest::Method::POST, token_endpoint, resource)?
        .form(form)
        .send()
        .map_err(|_| anyhow::anyhow!("OAuth token exchange failed"))?;
    let value = response_json(response)?;
    let access = field(&value, "access_token")?;
    ensure!(
        access.bytes().all(|b| (0x21 ..= 0x7e).contains(&b))
            && !access.contains("${")
            && value["token_type"].as_str().is_some_and(|v| v.eq_ignore_ascii_case("Bearer")),
        "OAuth requires a valid Bearer token"
    );
    let refresh = value["refresh_token"].as_str().map(str::to_owned).or(previous_refresh);
    ensure!(
        refresh
            .as_ref()
            .is_none_or(|v| !v.is_empty() && v.len() <= 8192 && !v.chars().any(char::is_control)),
        "Invalid OAuth refresh token"
    );
    let expires_at = match value.get("expires_in") {
        | None => None,
        | Some(value) => Some(now().saturating_add(value.as_u64().filter(|v| *v > 0).context("Invalid OAuth expires_in")?)),
    };
    Ok(Credential {
        identity: identity(server)?,
        resource: resource.into(),
        client_id: client_id.into(),
        token_endpoint: token_endpoint.into(),
        access_token: access.into(),
        refresh_token: refresh,
        expires_at,
    })
}
impl McpOAuth {
    pub fn cancel_all(&self) {
        if let Ok(attempts) = self.attempts.lock() {
            for attempt in attempts.values() {
                attempt.cancel.store(true, Ordering::SeqCst);
                if let Ok(mut state) = attempt.status.lock() {
                    *state = status(State::Cancelled);
                }
            }
        }
    }

    pub fn tokens(home: &Path, servers: &[McpServer]) -> Result<BTreeMap<String, String>> {
        let mut tokens = BTreeMap::new();
        for server in servers {
            if !matches!(
                server,
                McpServer::Http {
                    oauth: Some(_),
                    ..
                }
            ) {
                continue;
            }
            let _lock = lock(home)?;
            let mut saved = read(home, server)?.context("MCP OAuth login required; open MCP settings")?;
            if gpui_agent_domain::mcp_oauth_refresh_due(saved.expires_at, now(), saved.refresh_token.is_some()).map_err(anyhow::Error::msg)? {
                let refresh = saved.refresh_token.as_deref().context("MCP OAuth session expired; log in again")?;
                saved = credential(
                    server,
                    &saved.resource,
                    &saved.client_id,
                    &saved.token_endpoint,
                    &[
                        ("grant_type", "refresh_token"),
                        ("refresh_token", refresh),
                        ("client_id", &saved.client_id),
                        ("resource", &saved.resource),
                    ],
                    saved.refresh_token.clone(),
                )
                .map_err(|_| anyhow::anyhow!("MCP OAuth refresh failed; log in again"))?;
                save(home, server, &saved)?;
            }
            tokens.insert(server.name().into(), saved.access_token);
        }
        Ok(tokens)
    }

    pub fn execute(&self, home: &Path, server: &McpServer, command: Command) -> Result<Status> {
        gpui_agent_domain::validate_mcp(std::slice::from_ref(server)).map_err(anyhow::Error::msg)?;
        let McpServer::Http {
            url,
            oauth: Some(config),
            ..
        } = server
        else {
            bail!("OAuth is enabled only for HTTP MCP servers with oauth settings")
        };
        let key = path(home, server);
        let mut attempts = self.attempts.lock().map_err(|_| anyhow::anyhow!("OAuth registry unavailable"))?;
        if let Some(attempt) = attempts.get(&key) {
            if matches!(command, Command::Cancel | Command::Logout) {
                let _lock = lock(home)?;
                let mut current = attempt.status.lock().map_err(|_| anyhow::anyhow!("OAuth state unavailable"))?;
                if command == Command::Cancel && current.state == State::Connected {
                    return Ok(current.clone());
                }
                attempt.cancel.store(true, Ordering::SeqCst);
                *current = status(State::Cancelled);
            } else {
                let mut state = attempt.status.lock().map_err(|_| anyhow::anyhow!("OAuth state unavailable"))?;
                if state.state == State::Pending && attempt.deadline <= Instant::now() {
                    attempt.cancel.store(true, Ordering::SeqCst);
                    *state = status(State::Expired);
                }
                if state.state == State::Pending || (command == Command::Status && matches!(state.state, State::Cancelled | State::Failed | State::Expired)) {
                    return Ok(state.clone());
                }
            }
        }
        if command == Command::Cancel {
            return Ok(status(State::Cancelled));
        }
        if command == Command::Logout {
            let _lock = lock(home)?;
            match std::fs::remove_file(key) {
                | Ok(()) => {},
                | Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                | Err(e) => return Err(e.into()),
            };
            attempts.remove(&path(home, server));
            return Ok(status(State::SignedOut));
        }
        if command == Command::Status {
            let _lock = lock(home)?;
            let saved = read(home, server)?;
            return Ok(saved
                .map(|saved| Status {
                    state: if saved.expires_at.is_none_or(|at| at > now()) {
                        State::Connected
                    } else {
                        State::Expired
                    },
                    authorization_url: None,
                    expires_at: saved.expires_at,
                    message: None,
                })
                .unwrap_or_else(|| status(State::SignedOut)));
        }
        let profile_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(home.join(".operation.lock"))?;
        profile_lock
            .try_lock()
            .context("Provider profile is busy; finish or cancel its operation first")?;
        let discovery = discover(url)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let redirect = format!("http://{}/oauth/callback/{}", listener.local_addr()?, random()?);
        let client_id = if let Some(id) = &config.client_id {
            id.clone()
        } else {
            let registration =
                field(&discovery.metadata, "registration_endpoint").context("This server requires a pre-registered public client_id in oauth settings")?;
            let response=request(reqwest::Method::POST,registration,url)?.json(&json!({"client_name":"Gpui Agent MCP","redirect_uris":[redirect],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none"})).send().map_err(|_|anyhow::anyhow!("OAuth client registration failed"))?;
            let value = response_json(response)?;
            ensure!(
                value.get("client_secret").is_none() && value["token_endpoint_auth_method"].as_str().is_none_or(|v| v == "none"),
                "OAuth requires a public client without a client secret"
            );
            field(&value, "client_id")?.into()
        };
        let verifier = random()?;
        let state = random()?;
        let scopes = config
            .requested_scopes(discovery.challenge_scope.as_deref(), &discovery.scope)
            .map_err(anyhow::Error::msg)?;
        let mut authorization = endpoint(field(&discovery.metadata, "authorization_endpoint")?, url)?;
        ensure!(
            !authorization.query_pairs().any(|(key, _)| matches!(
                key.as_ref(),
                "response_type" | "client_id" | "redirect_uri" | "state" | "scope" | "resource" | "code_challenge" | "code_challenge_method"
            )),
            "OAuth authorization endpoint contains reserved parameters"
        );
        authorization.query_pairs_mut().extend_pairs([
            ("response_type", "code"),
            ("client_id", client_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("state", state.as_str()),
            ("code_challenge_method", "S256"),
            ("code_challenge", URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())).as_str()),
            ("resource", discovery.resource.as_str()),
        ]);
        if !scopes.is_empty() {
            authorization.query_pairs_mut().append_pair("scope", &scopes);
        }
        let result = Status {
            state: State::Pending,
            authorization_url: Some(authorization.into()),
            expires_at: Some(now() + 300),
            message: None,
        };
        let attempt = Arc::new(Attempt {
            cancel: AtomicBool::new(false),
            status: Mutex::new(result.clone()),
            deadline: Instant::now() + Duration::from_secs(300),
        });
        attempts.insert(key, attempt.clone());
        let home = home.to_owned();
        let server = server.clone();
        let resource = discovery.resource.clone();
        std::thread::spawn(move || {
            let _profile = profile_lock;
            let run = || -> Result<()> {
                loop {
                    if attempt.cancel.load(Ordering::SeqCst) {
                        return Ok(());
                    }
                    if Instant::now() >= attempt.deadline {
                        bail!("OAuth login timed out")
                    }
                    let (mut stream, _) = match listener.accept() {
                        | Ok(v) => v,
                        | Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(50));
                            continue;
                        },
                        | Err(e) => return Err(e.into()),
                    };
                    let callback = callback(
                        &mut stream,
                        &redirect,
                        &state,
                        &discovery.issuer,
                        discovery.metadata["authorization_response_iss_parameter_supported"] == true,
                    );
                    match callback {
                        | Ok(Some(code)) => {
                            let token = credential(
                                &server,
                                &resource,
                                &client_id,
                                field(&discovery.metadata, "token_endpoint")?,
                                &[
                                    ("grant_type", "authorization_code"),
                                    ("code", &code),
                                    ("client_id", &client_id),
                                    ("redirect_uri", &redirect),
                                    ("code_verifier", &verifier),
                                    ("resource", &resource),
                                ],
                                None,
                            );
                            let _lock = lock(&home)?;
                            if attempt.cancel.load(Ordering::SeqCst) {
                                reply(&mut stream, 400, "Login cancelled");
                                return Ok(());
                            }
                            match token {
                                | Ok(token) => {
                                    save(&home, &server, &token)?;
                                    reply(&mut stream, 200, "MCP login completed. You can close this window.");
                                    if let Ok(mut s) = attempt.status.lock() {
                                        *s = status(State::Connected)
                                    };
                                    return Ok(());
                                },
                                | Err(e) => {
                                    fail(&attempt);
                                    reply(&mut stream, 400, "MCP login failed. Return to the app and retry.");
                                    return Err(e);
                                },
                            }
                        },
                        | Ok(None) => {
                            fail(&attempt);
                            reply(&mut stream, 400, "MCP authorization was declined.");
                            bail!("MCP authorization was declined")
                        },
                        | Err(_) => reply(&mut stream, 400, "Invalid OAuth callback"),
                    }
                }
            };
            if run().is_err() {
                fail(&attempt);
            }
        });
        Ok(result)
    }
}
struct Discovery {
    resource: String,
    challenge_scope: Option<String>,
    metadata: Value,
    issuer: String,
    scope: String,
}
fn discover(resource: &str) -> Result<Discovery> {
    let resource_url = Url::parse(resource)?;
    let response = request(reqwest::Method::GET, resource, resource)?
        .header("Accept", "application/json")
        .send()
        .map_err(|_| anyhow::anyhow!("MCP OAuth discovery failed"))?;
    let mut challenge = BTreeMap::new();
    for value in response.headers().get_all("www-authenticate") {
        if let Ok(value) = value.to_str() {
            challenge.extend(gpui_agent_domain::mcp_bearer_challenge(value));
        }
    }
    let protected = if let Some(target) = challenge.get("resource_metadata") {
        get(target, resource)?
    } else {
        let mut target = resource_url.clone();
        target.set_path(&format!("/.well-known/oauth-protected-resource{}", resource_url.path().trim_end_matches('/')));
        get(target.as_str(), resource).or_else(|_| {
            target.set_path("/.well-known/oauth-protected-resource");
            get(target.as_str(), resource)
        })?
    };
    ensure!(
        Url::parse(field(&protected, "resource")?)? == resource_url,
        "OAuth resource metadata does not match the MCP server"
    );
    let issuer = protected["authorization_servers"]
        .as_array()
        .and_then(|v| v.first())
        .and_then(Value::as_str)
        .context("MCP resource has no authorization server")?;
    let issuer_url = endpoint(issuer, resource)?;
    let path = issuer_url.path().trim_end_matches('/');
    let mut candidates = vec![
        format!("/.well-known/oauth-authorization-server{path}"),
        format!("/.well-known/openid-configuration{path}"),
    ];
    if !path.is_empty() {
        candidates.push(format!("{path}/.well-known/openid-configuration"));
    }
    let mut metadata = None;
    for candidate in candidates {
        let mut target = issuer_url.clone();
        target.set_path(&candidate);
        if let Ok(value) = get(target.as_str(), resource) {
            metadata = Some(value);
            break;
        }
    }
    let metadata = metadata.context("OAuth authorization server discovery failed")?;
    ensure!(Url::parse(field(&metadata, "issuer")?)? == issuer_url, "OAuth issuer mismatch");
    ensure!(
        metadata["code_challenge_methods_supported"]
            .as_array()
            .is_some_and(|v| v.contains(&json!("S256"))),
        "OAuth server must support PKCE S256"
    );
    endpoint(field(&metadata, "authorization_endpoint")?, resource)?;
    endpoint(field(&metadata, "token_endpoint")?, resource)?;
    let scope = challenge.get("scope").cloned().unwrap_or_else(|| {
        protected["scopes_supported"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .filter(|v| *v != "offline_access")
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default()
    });
    gpui_agent_domain::McpOAuthConfig {
        client_id: None,
        scopes: scope.split_whitespace().map(str::to_owned).collect(),
    }
    .validate()
    .map_err(anyhow::Error::msg)?;
    Ok(Discovery {
        resource: field(&protected, "resource")?.into(),
        challenge_scope: challenge.get("scope").cloned(),
        metadata,
        issuer: issuer.into(),
        scope,
    })
}
fn callback(stream: &mut TcpStream, redirect: &str, state: &str, issuer: &str, issuer_required: bool) -> Result<Option<String>> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut bytes = Vec::new();
    let mut buf = [0; 1024];
    while !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
        let n = stream.read(&mut buf)?;
        ensure!(n > 0 && bytes.len() + n <= 8192, "Invalid callback");
        bytes.extend_from_slice(&buf[.. n]);
    }
    gpui_agent_domain::mcp_oauth_callback(std::str::from_utf8(&bytes)?, redirect, state, issuer, issuer_required).map_err(anyhow::Error::msg)
}
fn reply(stream: &mut TcpStream, code: u16, text: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {code} Result\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
        text.len()
    );
}
