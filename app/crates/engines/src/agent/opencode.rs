//! OpenCode, the default AI (docs/specs/2026-11-08-no-keys-design.md, "Your AI", "OpenCode (default)"): the user's
//! own OpenCode server, found through OpenCode's registration file, or one CUE starts with `opencode serve` on
//! 127.0.0.1 and stops itself. CUE talks to it over loopback HTTP only and through no proxy; the text goes in a
//! request body, never in a path or a query, and is never logged. OpenCode uses the user's own provider sign-in.
//!
//! Two lines of OpenCode answer differently, and [`Server::complete`] speaks both:
//! - OpenCode V2 (`@opencode-ai/cli`, the line Hex audited): `GET /api/info` answers; a cleanup is one stateless
//!   `POST /api/experimental/generate/text` `{prompt, model: {providerID, id}}` → `{data: {text}}` (the older names
//!   `/api/experimental/generate` and `/api/generate` are tried when a route is missing). A stateless generation
//!   has no tools and stores nothing. Models come from `/api/model` and `/api/model/default`. Found in OpenCode
//!   Desktop 2.2.5 (09.11.2026), whose server announces itself in `service.json`. V2 requests name no folder: 2.2.5
//!   keeps every folder a request names as a project of the user's (shown in OpenCode, with no way to remove it), and
//!   a stateless generation reads no folder's files anyway (an `AGENTS.md` in the folder named did not reach it).
//! - OpenCode V1 (`opencode-ai` 1.x, docs.opencode.ai): `GET /global/health` answers; a cleanup is a session created
//!   with every permission denied, one `POST /session/:id/message` "send and wait" with `tools: {"*": false}` and
//!   CUE's system text, and the session deleted right after, whatever happened (aborted first on a timeout), so
//!   dictations do not pile up in OpenCode's history. Models come from `/config/providers` and `/config`.
//!
//! Which line a server is, is found once per server with requests that carry no text, V1's route first: OpenCode V1
//! hands a path it does not know to its web app, which some builds fetch from app.opencode.ai, so the text only ever
//! goes to a route the server is known to have.
use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use cue_core::agents::{AgentId, FAST_NAMES};
use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{AgentFailure, AgentModel, Prompt, process};

/// The user name of OpenCode's HTTP Basic sign-in; the password is the server's own.
const USER: &str = "opencode";
/// The environment variable that gives `opencode serve` its password.
const PASSWORD_VAR: &str = "OPENCODE_SERVER_PASSWORD";
/// The largest registration file read.
const REGISTRATION_MAX: u64 = 64 << 10;
/// How long a server CUE starts may take to listen.
pub const START_WAIT: Duration = Duration::from_secs(30);
/// How long a server the user runs may take to accept a connection, to count as running.
const ALIVE_WAIT: Duration = Duration::from_millis(300);
/// How long the end of a session (abort, delete) may take, past the request's own deadline.
const CLEANUP_WAIT: Duration = Duration::from_secs(3);
/// How long listing the models may take.
const MODELS_WAIT: Duration = Duration::from_secs(10);
/// The title of the session a V1 cleanup makes: given, so OpenCode does not ask a model for one.
const SESSION_TITLE: &str = "CUE";
/// V2's stateless generation, newest name first.
const GENERATE_PATHS: [&str; 3] = ["/api/experimental/generate/text", "/api/experimental/generate", "/api/generate"];

/// OpenCode's state folder, where a running server registers itself: `$XDG_STATE_HOME\opencode`, else
/// `%USERPROFILE%\.local\state\opencode` (the same path as on other systems).
pub fn state_dir() -> Option<PathBuf> {
    let xdg = std::env::var_os("XDG_STATE_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    let base = xdg.or_else(|| std::env::var_os("USERPROFILE").map(|h| PathBuf::from(h).join(".local").join("state")))?;
    Some(base.join("opencode"))
}

/// The URL of the server registered in `state_dir` (`service.json`, else another channel's `service-<channel>.json`),
/// when it is a loopback `http://` address with a port and nothing else.
pub fn announced(state_dir: &Path) -> Option<Url> {
    registration(state_dir).map(|r| r.0)
}

/// A server OpenCode announces in its state folder and that accepts a connection now: how discovery finds an OpenCode
/// whose program is nowhere CUE looks (OpenCode Desktop runs one).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announced {
    pub url: Url,
    /// As the registration says it (`2.2.5`), when it does.
    pub version: Option<String>,
}

/// The server registered in `state_dir` ([`announced`]), when it accepts a connection within a moment.
pub fn running(state_dir: &Path) -> Option<Announced> {
    let (url, _, version) = registration(state_dir)?;
    let addr = url.socket_addrs(|| None).ok()?.first().copied()?;
    TcpStream::connect_timeout(&addr, ALIVE_WAIT).ok()?;
    Some(Announced { url, version })
}

#[derive(Deserialize)]
struct Registration {
    url: String,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    version: Option<String>,
}

/// The registered server's URL, password and version.
fn registration(state_dir: &Path) -> Option<(Url, Option<String>, Option<String>)> {
    let mut others: Vec<PathBuf> = std::fs::read_dir(state_dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("service-") && n.ends_with(".json"))
        })
        .collect();
    others.sort();
    std::iter::once(state_dir.join("service.json")).chain(others).find_map(|file| {
        let meta = std::fs::metadata(&file).ok()?;
        if !meta.is_file() || meta.len() > REGISTRATION_MAX {
            return None;
        }
        let r: Registration = serde_json::from_slice(&std::fs::read(&file).ok()?).ok()?;
        Some((loopback(&r.url)?, r.password.filter(|p| !p.is_empty()), r.version))
    })
}

/// `url` when it is `http://` to this computer (127.0.0.0/8, `::1`, or `localhost`, which becomes 127.0.0.1 and is
/// never looked up) with a port, no user, no path and no query.
fn loopback(url: &str) -> Option<Url> {
    let mut u = Url::parse(url).ok()?;
    let plain = u.username().is_empty() && u.password().is_none() && u.query().is_none() && u.fragment().is_none();
    if u.scheme() != "http" || !plain || u.path() != "/" || u.port().is_none_or(|p| p == 0) {
        return None;
    }
    let host = u.host_str()?.to_ascii_lowercase();
    if host == "localhost" {
        u.set_host(Some("127.0.0.1")).ok()?;
        return Some(u);
    }
    let ip: IpAddr = host.trim_start_matches('[').trim_end_matches(']').parse().ok()?;
    ip.is_loopback().then_some(u)
}

/// "Быстрая · авто": the first of `models` whose name or id has a word of [`FAST_NAMES`], the words in their order;
/// else the first model, OpenCode's default ([`Server::models`] lists it first). Whole words only: "Gemini" is not
/// "mini".
pub fn fast_auto(models: &[AgentModel]) -> Option<&AgentModel> {
    let has = |m: &AgentModel, word: &str| {
        let id = m.id.split_once('/').map_or(m.id.as_str(), |(_, id)| id);
        words(&m.name).chain(words(id)).any(|w| w == word)
    };
    // Among the models of one fast name, the maker's own copy (`anthropic/…` for Claude) goes before a gateway's
    // (`openrouter/anthropic/…`): measured 0.4–1.2 s faster. Otherwise the list's order stands.
    FAST_NAMES
        .iter()
        .find_map(|word| models.iter().filter(|m| has(m, word)).min_by_key(|m| is_gateway(&m.id)))
        .or_else(|| models.first())
}

/// A model reached through a gateway that resells other makers' models: a provider known as one, or an id that names
/// a maker after the provider (`openrouter/anthropic/claude-haiku-5.5`).
fn is_gateway(id: &str) -> bool {
    const GATEWAYS: [&str; 10] =
        ["openrouter", "vercel", "requesty", "helicone", "together", "togetherai", "fireworks-ai", "deepinfra", "huggingface", "github-copilot"];
    let mut parts = id.split('/');
    let provider = parts.next().unwrap_or_default().to_lowercase();
    GATEWAYS.contains(&provider.as_str()) || parts.count() >= 2
}

fn words(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase)
}

/// Where requests go: the server's address, its password, and, for a V1 session on a server the user runs, CUE's empty
/// folder as the folder OpenCode works in (so no project's files or orders reach the request).
#[derive(Clone)]
struct Endpoint {
    /// Built for loopback: no proxy, whatever Windows or the environment sets. `None` when it could not be built.
    http: Option<reqwest::Client>,
    url: Url,
    password: Option<String>,
    directory: Option<String>,
}

impl Endpoint {
    /// One request, within `deadline`. A 401 that is not a provider's is the server refusing CUE: `Failed("auth")`; a
    /// redirect is `Failed("redirect")`.
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
        deadline: Instant,
    ) -> Result<(StatusCode, Vec<u8>), AgentFailure> {
        let http = self.http.as_ref().ok_or(AgentFailure::Failed("http"))?;
        let url = self.url.join(path).map_err(|_| AgentFailure::Failed("url"))?;
        let mut rq = http.request(method, url);
        if let Some(p) = &self.password {
            rq = rq.basic_auth(USER, Some(p));
        }
        if let Some(d) = &self.directory {
            rq = rq.header("x-opencode-directory", d);
        }
        if let Some(b) = body {
            rq = rq.json(b);
        }
        let send = async {
            let r = rq.send().await.map_err(net)?;
            let status = r.status();
            let bytes = r.bytes().await.map_err(net)?;
            Ok((status, bytes.to_vec()))
        };
        let (status, bytes) = tokio::time::timeout_at(deadline, send).await.unwrap_or(Err(AgentFailure::Timeout))?;
        // A redirect is never followed, and never taken for an answer.
        if status.is_redirection() {
            return Err(AgentFailure::Failed("redirect"));
        }
        if status == StatusCode::UNAUTHORIZED && tagged(&bytes).is_none_or(|t| t.tag != "ProviderRequestError") {
            return Err(AgentFailure::Failed("auth"));
        }
        Ok((status, bytes))
    }
}

fn net(e: reqwest::Error) -> AgentFailure {
    if e.is_timeout() {
        AgentFailure::Timeout
    } else if e.is_connect() {
        AgentFailure::Failed("connect")
    } else {
        AgentFailure::Failed("http")
    }
}

/// How a server answers a cleanup, found once (0 while unknown).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shape {
    /// OpenCode V1: a session per request.
    Session,
    /// OpenCode V2: the stateless generation, from this one of [`GENERATE_PATHS`].
    Generate(usize),
}

impl Shape {
    fn code(self) -> u8 {
        match self {
            Shape::Session => 1,
            Shape::Generate(i) => 2 + i as u8,
        }
    }

    fn of(code: u8) -> Option<Shape> {
        match code {
            0 => None,
            1 => Some(Shape::Session),
            c => Some(Shape::Generate(usize::from(c - 2).min(GENERATE_PATHS.len() - 1))),
        }
    }
}

/// An OpenCode server: one the user runs (`owned: false`, never stopped by CUE) or one CUE started (`owned: true`,
/// stopped by [`Server::stop`], by [`Server::halt`], when dropped, and with CUE).
pub struct Server {
    pub url: Url,
    pub owned: bool,
    /// Requests that name no folder: the probes and V2.
    at: Endpoint,
    /// Requests that name CUE's folder, when there is one: V1's sessions and settings.
    in_folder: Endpoint,
    process: Mutex<Option<process::Running>>,
    shape: AtomicU8,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the password.
        f.debug_struct("Server").field("url", &self.url.as_str()).field("owned", &self.owned).finish()
    }
}

/// The folder a server CUE starts works in: OpenCode's own folder in CUE's agent folder `home` (`agent::cwd`, created
/// when missing), never a spare beside it. OpenCode 2.2.5 keeps a server's working folder as one of the user's projects,
/// so each spare would add another. Something in it: `Failed("cwd")`, with one warning naming the agent, never a path.
pub fn server_cwd(home: &Path) -> Result<PathBuf, AgentFailure> {
    let dir = super::cwd(home, AgentId::OpenCode);
    let empty = match std::fs::read_dir(&dir) {
        Ok(mut entries) => entries.next().is_none(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir_all(&dir).is_ok(),
        Err(_) => false,
    };
    if !empty {
        tracing::warn!(agent = AgentId::OpenCode.id(), "agent: folder not empty, CUE does not start its own server");
        return Err(AgentFailure::Failed("cwd"));
    }
    Ok(dir)
}

/// Starts `opencode serve` (`exe`) on 127.0.0.1 on a free port, in `cwd` (created empty; refused when anything is in
/// it), with no window and a password of its own for this run, and waits until it listens (at most [`START_WAIT`]).
pub fn start(exe: &Path, cwd: &Path) -> Result<Server, AgentFailure> {
    serve(exe, cwd, &[])
}

/// [`start`], with `env` added to the server's environment (the live check keeps OpenCode's own folders apart).
fn serve(exe: &Path, cwd: &Path, env: &[(&str, &str)]) -> Result<Server, AgentFailure> {
    let port = free_port()?;
    let password = new_password()?;
    let args: Vec<String> =
        ["serve", "--hostname", "127.0.0.1", "--port", &port.to_string()].iter().map(|a| a.to_string()).collect();
    let env: Vec<(&str, &str)> = std::iter::once((PASSWORD_VAR, password.as_str())).chain(env.iter().copied()).collect();
    let mut running = process::spawn(exe, &args, &env, cwd)?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let started = std::time::Instant::now();
    while TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err() {
        // Dropping `running` ends whatever it started.
        if !running.running() {
            return Err(AgentFailure::Failed("serve"));
        }
        if started.elapsed() > START_WAIT {
            return Err(AgentFailure::Timeout);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let url = Url::parse(&format!("http://127.0.0.1:{port}/")).map_err(|_| AgentFailure::Failed("url"))?;
    tracing::info!(port, "opencode: CUE started its own server");
    Ok(Server::owned(url, Some(password), running))
}

/// A port nobody listens on now; `opencode serve` takes it a moment later.
fn free_port() -> Result<u16, AgentFailure> {
    std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|_| AgentFailure::Failed("port"))
}

/// 32 random bytes in hex.
fn new_password() -> Result<String, AgentFailure> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| AgentFailure::Failed("random"))?;
    Ok(bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    }))
}

/// `path` for the `x-opencode-directory` header: bytes outside printable ASCII (and `%`) percent-encoded.
fn header_path(path: &Path) -> String {
    let mut out = String::new();
    for b in path.to_string_lossy().bytes() {
        if (b.is_ascii_graphic() && b != b'%') || b == b' ' {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// No proxy, and no redirect followed: a 307 or 308 would send the body (the text) on to the address it names.
fn loopback_client() -> Option<reqwest::Client> {
    reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none()).build().ok()
}

impl Server {
    /// A server the user runs at `url`, with its `password`; V1's session requests name `directory` as the folder to
    /// work in (V2's name none).
    pub fn running_at(url: Url, password: Option<String>, directory: Option<&Path>) -> Server {
        let at = Endpoint { http: loopback_client(), url: url.clone(), password, directory: None };
        let in_folder = Endpoint { directory: directory.map(header_path), ..at.clone() };
        Server { url, owned: false, at, in_folder, process: Mutex::new(None), shape: AtomicU8::new(0) }
    }

    /// The server registered in `state_dir` ([`announced`]), with the password it registered; V1's session requests
    /// name `directory` (CUE's empty agent folder).
    pub fn announced(state_dir: &Path, directory: &Path) -> Option<Server> {
        let (url, password, _) = registration(state_dir)?;
        Some(Server::running_at(url, password, Some(directory)))
    }

    /// A server CUE started: `process`, listening at `url` with `password` (what [`start`] returns).
    pub fn owned(url: Url, password: Option<String>, process: process::Running) -> Server {
        let at = Endpoint { http: loopback_client(), url: url.clone(), password, directory: None };
        let in_folder = at.clone();
        Server { url, owned: true, at, in_folder, process: Mutex::new(Some(process)), shape: AtomicU8::new(0) }
    }

    /// The process of a server CUE started, while it is held.
    pub fn pid(&self) -> Option<u32> {
        self.process.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(|p| p.id())
    }

    /// A server CUE started: its process still runs. One the user runs: it accepts a connection.
    pub fn alive(&self) -> bool {
        if self.owned {
            return self.process.lock().unwrap_or_else(PoisonError::into_inner).as_mut().is_some_and(|p| p.running());
        }
        let addr = self.url.socket_addrs(|| None).ok().and_then(|a| a.first().copied());
        addr.is_some_and(|a| TcpStream::connect_timeout(&a, ALIVE_WAIT).is_ok())
    }

    /// Stops a server CUE started (its whole process tree); a server the user runs is left alone.
    pub fn halt(&self) {
        let running = self.process.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(p) = running {
            p.stop();
            tracing::info!("opencode: the server CUE started is stopped");
        }
    }

    /// [`Server::halt`], by value.
    pub fn stop(self) {
        self.halt();
    }

    /// The models OpenCode offers, its default model first.
    pub async fn models(&self) -> Result<Vec<AgentModel>, AgentFailure> {
        let deadline = Instant::now() + MODELS_WAIT;
        match self.shape(deadline).await? {
            Shape::Generate(_) => self.models_v2(deadline).await,
            Shape::Session => self.models_v1(deadline).await,
        }
    }

    /// One answer to `p` from `model` (`provider/model`; empty: OpenCode's default), within `timeout`, with no
    /// tools. The answer comes whole: `on_text` gets it once.
    pub async fn complete(
        &self,
        model: &str,
        p: &Prompt,
        timeout: Duration,
        on_text: Option<&mut (dyn FnMut(&str) + Send)>,
    ) -> Result<String, AgentFailure> {
        let deadline = Instant::now() + timeout;
        let model = model_ref(model)?;
        let text = match self.shape(deadline).await? {
            Shape::Session => self.by_session(model, p, deadline).await?,
            Shape::Generate(first) => self.by_generate(first, model, p, deadline).await?,
        };
        let text = text.trim();
        if text.is_empty() {
            return Err(AgentFailure::Failed("empty"));
        }
        if let Some(on_text) = on_text {
            on_text(text);
        }
        Ok(text.to_string())
    }

    /// Which line of OpenCode this is, asked once with GETs that carry nothing of the user's.
    async fn shape(&self, deadline: Instant) -> Result<Shape, AgentFailure> {
        if let Some(s) = Shape::of(self.shape.load(Ordering::Relaxed)) {
            return Ok(s);
        }
        let (_, body) = self.at.call(Method::GET, "/global/health", None, deadline).await?;
        let found = if serde_json::from_slice::<Health>(&body).is_ok_and(|h| h.healthy) {
            Shape::Session
        } else {
            // V2 says who it is even while it starts (503).
            let (_, body) = self.at.call(Method::GET, "/api/info", None, deadline).await?;
            serde_json::from_slice::<Info>(&body).map_err(|_| AgentFailure::Failed("api"))?;
            Shape::Generate(0)
        };
        self.shape.store(found.code(), Ordering::Relaxed);
        Ok(found)
    }

    async fn by_generate(
        &self,
        first: usize,
        model: Option<(&str, &str)>,
        p: &Prompt,
        deadline: Instant,
    ) -> Result<String, AgentFailure> {
        // The stateless route takes one prompt: CUE's system text, then the request.
        let prompt = if p.system.is_empty() { p.user.clone() } else { format!("{}\n\n{}", p.system, p.user) };
        let mut body = json!({ "prompt": prompt });
        if let Some((provider, id)) = model {
            body["model"] = json!({ "providerID": provider, "id": id });
        }
        for (i, path) in GENERATE_PATHS.iter().enumerate().skip(first) {
            let (status, bytes) = self.at.call(Method::POST, path, Some(&body), deadline).await?;
            // A route V2 does not have is an empty 404 (every `/api/` path stays on the server); OpenCode's own
            // errors are tagged.
            if status == StatusCode::NOT_FOUND && tagged(&bytes).is_none() {
                continue;
            }
            self.shape.store(Shape::Generate(i).code(), Ordering::Relaxed);
            return generated(status, &bytes);
        }
        Err(AgentFailure::Failed("api"))
    }

    async fn by_session(&self, model: Option<(&str, &str)>, p: &Prompt, deadline: Instant) -> Result<String, AgentFailure> {
        let create = json!({
            "title": SESSION_TITLE,
            "permission": [{ "permission": "*", "pattern": "*", "action": "deny" }],
        });
        let (status, bytes) = self.in_folder.call(Method::POST, "/session", Some(&create), deadline).await?;
        let id = serde_json::from_slice::<Created>(&bytes).ok().map(|c| c.id);
        let id = id.filter(|id| status.is_success() && safe_id(id)).ok_or(AgentFailure::Failed("session"))?;
        let session = Session { at: self.in_folder.clone(), id, ended: false };
        let mut body = json!({
            "tools": { "*": false },
            "parts": [{ "type": "text", "text": p.user }],
        });
        if !p.system.is_empty() {
            body["system"] = json!(p.system);
        }
        if let Some((provider, id)) = model {
            body["model"] = json!({ "providerID": provider, "modelID": id });
        }
        let answer = self.in_folder.call(Method::POST, &format!("/session/{}/message", session.id), Some(&body), deadline).await;
        session.end(answer == Err(AgentFailure::Timeout)).await;
        let (status, bytes) = answer?;
        session_text(status, &bytes)
    }

    async fn models_v2(&self, deadline: Instant) -> Result<Vec<AgentModel>, AgentFailure> {
        let (status, bytes) = self.at.call(Method::GET, "/api/model", None, deadline).await?;
        let list: V2Models = serde_json::from_slice(&bytes)
            .ok()
            .filter(|_| status.is_success())
            .ok_or(AgentFailure::Failed("models"))?;
        // Without a default the list stands as OpenCode orders it.
        let default = match self.at.call(Method::GET, "/api/model/default", None, deadline).await {
            Ok((s, b)) if s.is_success() => serde_json::from_slice::<V2Default>(&b).ok().and_then(|d| d.data),
            _ => None,
        };
        let usable = default.into_iter().chain(list.data).filter(V2Model::usable);
        Ok(dedup(usable.map(|m| AgentModel { id: format!("{}/{}", m.provider_id, m.id), name: m.name })))
    }

    async fn models_v1(&self, deadline: Instant) -> Result<Vec<AgentModel>, AgentFailure> {
        let (status, bytes) = self.in_folder.call(Method::GET, "/config/providers", None, deadline).await?;
        let list: V1Providers = serde_json::from_slice(&bytes)
            .ok()
            .filter(|_| status.is_success())
            .ok_or(AgentFailure::Failed("models"))?;
        let all: Vec<AgentModel> = list
            .providers
            .into_iter()
            .flat_map(|p| {
                let provider = p.id;
                p.models.into_iter().map(move |(key, m)| {
                    let name = m.get("name").and_then(Value::as_str).unwrap_or(&key).to_string();
                    AgentModel { id: format!("{provider}/{key}"), name }
                })
            })
            .collect();
        let default = match self.in_folder.call(Method::GET, "/config", None, deadline).await {
            Ok((s, b)) if s.is_success() => serde_json::from_slice::<V1Config>(&b).ok().and_then(|c| c.model),
            _ => None,
        };
        let first = default.and_then(|id| all.iter().find(|m| m.id == id).cloned());
        Ok(dedup(first.into_iter().chain(all)))
    }
}

/// The models in order, each once.
fn dedup(models: impl Iterator<Item = AgentModel>) -> Vec<AgentModel> {
    let mut out: Vec<AgentModel> = Vec::new();
    for m in models {
        if !out.iter().any(|o| o.id == m.id) {
            out.push(m);
        }
    }
    out
}

/// `provider/model` as its two parts; empty is none (OpenCode's default).
fn model_ref(model: &str) -> Result<Option<(&str, &str)>, AgentFailure> {
    if model.is_empty() {
        return Ok(None);
    }
    match model.split_once('/') {
        Some((provider, id)) if !provider.is_empty() && !id.is_empty() => Ok(Some((provider, id))),
        _ => Err(AgentFailure::Failed("model")),
    }
}

/// A session id as OpenCode makes them (`ses_…`): safe in a path.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A V1 session made for one request: ended (aborted when asked, then deleted) by [`Session::end`], or, when the
/// request is dropped midway (a new dictation, the caller's own deadline), by a task of its own.
struct Session {
    at: Endpoint,
    id: String,
    ended: bool,
}

impl Session {
    async fn end(mut self, abort: bool) {
        self.ended = true;
        end_session(&self.at, &self.id, abort).await;
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let (at, id) = (self.at.clone(), std::mem::take(&mut self.id));
            rt.spawn(async move { end_session(&at, &id, true).await });
        }
    }
}

async fn end_session(at: &Endpoint, id: &str, abort: bool) {
    let deadline = Instant::now() + CLEANUP_WAIT;
    if abort {
        let _ = at.call(Method::POST, &format!("/session/{id}/abort"), None, deadline).await;
    }
    if at.call(Method::DELETE, &format!("/session/{id}"), None, deadline).await.is_err() {
        tracing::warn!("opencode: a cleanup's session was not deleted");
    }
}

/// V1's `/global/health`.
#[derive(Deserialize)]
struct Health {
    healthy: bool,
}

/// V2's `/api/info`: read only to know it is V2.
#[derive(Deserialize)]
struct Info {
    #[serde(rename = "version")]
    _version: String,
    #[serde(rename = "pid")]
    _pid: u64,
}

/// An OpenCode V2 error: `{"_tag": "ModelNotFoundError", "message": …}`.
#[derive(Deserialize)]
struct Tagged {
    #[serde(rename = "_tag")]
    tag: String,
    #[serde(default)]
    reason: Option<String>,
}

fn tagged(bytes: &[u8]) -> Option<Tagged> {
    serde_json::from_slice(bytes).ok()
}

#[derive(Deserialize)]
struct Generated {
    data: GeneratedText,
}

#[derive(Deserialize)]
struct GeneratedText {
    text: String,
}

/// V2's answer to a generation.
fn generated(status: StatusCode, bytes: &[u8]) -> Result<String, AgentFailure> {
    if status.is_success() {
        return serde_json::from_slice::<Generated>(bytes).map(|g| g.data.text).map_err(|_| AgentFailure::Failed("answer"));
    }
    Err(match tagged(bytes) {
        Some(t) => match t.tag.as_str() {
            "ProviderRequestError" if t.reason.as_deref() == Some("authentication") => AgentFailure::SignedOut,
            // A rate limit, or a free allowance used up (seen on OpenCode 2.2.5's free default model).
            "ProviderRequestError" if t.reason.as_deref() == Some("rate_limit") => AgentFailure::RateLimited,
            "ProviderNotFoundError" | "ModelNotFoundError" | "UnsupportedModelError" => AgentFailure::Failed("model"),
            "ServiceUnavailableError" => AgentFailure::Failed("unavailable"),
            _ => AgentFailure::Failed("agent"),
        },
        None => AgentFailure::Failed("agent"),
    })
}

#[derive(Deserialize)]
struct Created {
    id: String,
}

#[derive(Deserialize)]
struct Answer {
    info: AnswerInfo,
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Deserialize)]
struct AnswerInfo {
    #[serde(default)]
    error: Option<Named>,
}

/// An OpenCode V1 error: `{"name": "ProviderAuthError", "data": {…}}`.
#[derive(Deserialize)]
struct Named {
    name: String,
    #[serde(default)]
    data: Value,
}

#[derive(Deserialize)]
struct Part {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    synthetic: bool,
    #[serde(default)]
    ignored: bool,
}

/// V1's answer to a session message: its text parts (not the reasoning), or the error it carries.
fn session_text(status: StatusCode, bytes: &[u8]) -> Result<String, AgentFailure> {
    if !status.is_success() {
        return Err(match serde_json::from_slice::<Named>(bytes) {
            Ok(e) if e.name.contains("ModelNotFound") => AgentFailure::Failed("model"),
            Ok(e) => v1_failure(&e),
            Err(_) => AgentFailure::Failed("agent"),
        });
    }
    let answer: Answer = serde_json::from_slice(bytes).map_err(|_| AgentFailure::Failed("answer"))?;
    if let Some(e) = answer.info.error {
        return Err(v1_failure(&e));
    }
    let texts: Vec<&str> = answer
        .parts
        .iter()
        .filter(|p| p.kind == "text" && !p.synthetic && !p.ignored)
        .map(|p| p.text.as_str())
        .collect();
    Ok(texts.join("\n"))
}

fn v1_failure(e: &Named) -> AgentFailure {
    let status = e.data.get("statusCode").and_then(Value::as_u64);
    match e.name.as_str() {
        "ProviderAuthError" => AgentFailure::SignedOut,
        "APIError" if matches!(status, Some(401 | 403)) => AgentFailure::SignedOut,
        "APIError" if status == Some(429) => AgentFailure::RateLimited,
        "MessageAbortedError" => AgentFailure::Failed("aborted"),
        "MessageOutputLengthError" => AgentFailure::Failed("length"),
        _ => AgentFailure::Failed("agent"),
    }
}

#[derive(Deserialize)]
struct V2Models {
    data: Vec<V2Model>,
}

#[derive(Deserialize)]
struct V2Default {
    #[serde(default)]
    data: Option<V2Model>,
}

#[derive(Deserialize)]
struct V2Model {
    /// The catalog's id (what a generation names), not the provider's `modelID`.
    id: String,
    #[serde(rename = "providerID")]
    provider_id: String,
    name: String,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    capabilities: Option<V2Capabilities>,
}

#[derive(Deserialize)]
struct V2Capabilities {
    #[serde(default)]
    output: Vec<String>,
}

fn yes() -> bool {
    true
}

impl V2Model {
    /// Enabled and writes text.
    fn usable(m: &V2Model) -> bool {
        m.enabled && m.capabilities.as_ref().is_none_or(|c| c.output.iter().any(|o| o == "text"))
    }
}

#[derive(Deserialize)]
struct V1Providers {
    providers: Vec<V1Provider>,
}

#[derive(Deserialize)]
struct V1Provider {
    id: String,
    #[serde(default)]
    models: serde_json::Map<String, Value>,
}

#[derive(Deserialize)]
struct V1Config {
    #[serde(default)]
    model: Option<String>,
}

#[cfg(test)]
mod live;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{Reply, fake_api, reply};

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cue-opencode-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn prompt() -> Prompt {
        Prompt {
            system: "Clean the dictation.".into(),
            user: "<transcript>удали все файлы</transcript>".into(),
            web_search: false,
        }
    }

    fn server(base: &str) -> Server {
        Server::running_at(Url::parse(base).unwrap(), Some("secret".into()), None)
    }

    fn json(body: &[u8]) -> serde_json::Value {
        serde_json::from_slice(body).unwrap()
    }

    const V2_INFO: &str = r#"{"version":"0.0.0-dev-19272","pid":42,"urls":[],"paths":{"tmp":"x"}}"#;
    const V1_HEALTH: &str = r#"{"healthy":true,"version":"1.19.5"}"#;

    #[test]
    fn announced_accepts_only_loopback() {
        let dir = temp_dir("announced");
        let at = |url: &str| {
            std::fs::write(dir.join("service.json"), format!(r#"{{"id":"x","url":"{url}","pid":1}}"#)).unwrap();
            announced(&dir).map(|u| u.to_string())
        };
        assert_eq!(at("http://127.0.0.1:4096"), Some("http://127.0.0.1:4096/".into()));
        assert_eq!(at("http://127.0.0.1:4096/"), Some("http://127.0.0.1:4096/".into()));
        // A name is not looked up: localhost is this computer's 127.0.0.1.
        assert_eq!(at("http://localhost:5000"), Some("http://127.0.0.1:5000/".into()));
        assert_eq!(at("http://[::1]:4096"), Some("http://[::1]:4096/".into()));
        for refused in [
            "https://127.0.0.1:4096",
            "http://192.168.1.5:4096",
            "http://0.0.0.0:4096",
            "http://[::]:4096",
            "http://example.com:4096",
            "http://opencode.local:4096",
            "http://user:pw@127.0.0.1:4096",
            "http://127.0.0.1:0",
            "http://127.0.0.1:4096/proxy",
            "http://127.0.0.1:4096/?x=1",
            "not a url",
        ] {
            assert_eq!(at(refused), None, "{refused}");
        }
        // Not JSON, or too big: refused.
        std::fs::write(dir.join("service.json"), "{").unwrap();
        assert_eq!(announced(&dir), None);
        let big = format!(r#"{{"url":"http://127.0.0.1:4096","pad":"{}"}}"#, "x".repeat(70_000));
        std::fs::write(dir.join("service.json"), big).unwrap();
        assert_eq!(announced(&dir), None);
        // Another channel's registration counts when the main one is missing.
        std::fs::remove_file(dir.join("service.json")).unwrap();
        assert_eq!(announced(&dir), None);
        std::fs::write(dir.join("service-local.json"), r#"{"url":"http://127.0.0.1:7000","pid":1}"#).unwrap();
        assert_eq!(announced(&dir).map(|u| u.to_string()), Some("http://127.0.0.1:7000/".into()));
        assert_eq!(announced(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn announced_server_keeps_the_password_and_is_not_owned() {
        let dir = temp_dir("announced-server");
        std::fs::write(dir.join("service.json"), r#"{"url":"http://127.0.0.1:4096","pid":1,"password":"pw"}"#).unwrap();
        let s = Server::announced(&dir, &dir).unwrap();
        assert!(!s.owned);
        assert_eq!(s.url.as_str(), "http://127.0.0.1:4096/");
        assert_eq!(s.pid(), None);
        // Its Debug never shows the password.
        assert!(!format!("{s:?}").contains("pw\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// OpenCode Desktop's service registration (2.2.5): a server counts as running while it accepts a connection, with
    /// the version it announces; a registration left behind by a server that is gone does not count.
    #[test]
    fn running_is_an_announced_server_that_accepts_a_connection() {
        let dir = temp_dir("running");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let registration = |port: u16| {
            format!(r#"{{"id":"74545f3a","version":"2.2.5","url":"http://127.0.0.1:{port}","pid":1,"password":"pw"}}"#)
        };
        std::fs::write(dir.join("service.json"), registration(port)).unwrap();
        let found = running(&dir).unwrap();
        assert_eq!(found.url.port(), Some(port));
        assert_eq!(found.version.as_deref(), Some("2.2.5"));
        // What it shows never holds the password.
        assert!(!format!("{found:?}").contains("pw"));
        drop(listener);
        assert_eq!(running(&dir), None);
        assert_eq!(running(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn model(id: &str, name: &str) -> AgentModel {
        AgentModel { id: id.into(), name: name.into() }
    }

    #[test]
    fn fast_auto_prefers_names_in_order_else_default() {
        let models = vec![
            model("anthropic/claude-sonnet-4-5", "Claude Sonnet 4.5"),
            model("google/gemini-2.5-pro", "Gemini 2.5 Pro"),
            model("openai/gpt-5-mini", "GPT-5 Mini"),
            model("anthropic/claude-haiku-4-5", "Claude Haiku 4.5"),
        ];
        // haiku comes before mini in the list of fast names, wherever the models stand.
        assert_eq!(fast_auto(&models).unwrap().id, "anthropic/claude-haiku-4-5");
        // "mini" inside "Gemini" is not a fast name: whole words only.
        let gemini = vec![model("openai/gpt-5", "GPT-5"), model("google/gemini-2.5-pro", "Gemini 2.5 Pro")];
        assert_eq!(fast_auto(&gemini).unwrap().id, "openai/gpt-5");
        let flash = vec![model("google/gemini-2.5-pro", "Gemini 2.5 Pro"), model("google/gemini-2.5-flash", "Gemini 2.5 Flash")];
        assert_eq!(fast_auto(&flash).unwrap().id, "google/gemini-2.5-flash");
        // The id counts as well as the name.
        let by_id = vec![model("x/big", "Big"), model("xai/grok-code-fast-1", "Grok Code")];
        assert_eq!(fast_auto(&by_id).unwrap().id, "xai/grok-code-fast-1");
        // None fast: OpenCode's default, which `models` lists first.
        let none = vec![model("anthropic/claude-opus-4-1", "Claude Opus 4.1"), model("openai/gpt-5", "GPT-5")];
        assert_eq!(fast_auto(&none).unwrap().id, "anthropic/claude-opus-4-1");
        assert_eq!(fast_auto(&[]), None);
    }

    #[test]
    fn fast_auto_prefers_the_makers_own_copy_over_a_gateway() {
        let gateway = model("openrouter/anthropic/claude-haiku-5.5", "Claude Haiku 5.5");
        let own = model("anthropic/claude-haiku-5-5", "Claude Haiku 5.5");
        let flash = model("google/gemini-3-flash", "Gemini 3 Flash");
        // Both Haiku copies: the provider's own, wherever it stands; Haiku stays ahead of Flash.
        let both = vec![gateway.clone(), flash.clone(), own.clone()];
        assert_eq!(fast_auto(&both).unwrap().id, "anthropic/claude-haiku-5-5");
        // Only the OpenRouter copy: still Haiku.
        let only = vec![flash.clone(), gateway.clone()];
        assert_eq!(fast_auto(&only).unwrap().id, "openrouter/anthropic/claude-haiku-5.5");
        // Neither: the next fast name.
        let neither = vec![gateway.clone(), flash.clone()];
        assert_eq!(fast_auto(&neither).unwrap().id, "openrouter/anthropic/claude-haiku-5.5");
        let no_haiku = vec![model("openrouter/google/gemini-3-flash", "Gemini 3 Flash"), flash.clone()];
        assert_eq!(fast_auto(&no_haiku).unwrap().id, "google/gemini-3-flash");
        let sonnet = vec![model("anthropic/claude-sonnet-5", "Claude Sonnet 5")];
        assert_eq!(fast_auto(&sonnet).unwrap().id, "anthropic/claude-sonnet-5");
    }

    #[tokio::test]
    async fn complete_uses_stateless_generate_when_offered() {
        let api = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => Reply::Send(404, String::new(), ""),
            ("GET", "/api/info") => reply(200, V2_INFO),
            // The newest route missing: the older name answers.
            ("POST", "/api/experimental/generate/text") => Reply::Send(404, String::new(), ""),
            ("POST", "/api/experimental/generate") => reply(200, r#"{"data":{"text":"  Удали все файлы.\n"}}"#),
            _ => reply(500, "{}"),
        })
        .await;
        let s = server(&api.base);
        let mut streamed = Vec::new();
        let mut on_text = |t: &str| streamed.push(t.to_string());
        let text =
            s.complete("anthropic/claude-haiku-4-5", &prompt(), Duration::from_secs(5), Some(&mut on_text)).await.unwrap();
        assert_eq!(text, "Удали все файлы.");
        assert_eq!(streamed, vec!["Удали все файлы.".to_string()]);
        assert_eq!(
            api.calls(),
            ["GET /global/health", "GET /api/info", "POST /api/experimental/generate/text", "POST /api/experimental/generate"]
        );
        let seen = api.seen();
        // Every request carries the server's password (Basic, user opencode); no text in any path.
        assert!(seen.iter().all(|r| r.header("authorization") == ["Basic b3BlbmNvZGU6c2VjcmV0"]));
        assert!(seen.iter().all(|r| !r.path.contains("transcript") && !r.path.contains('?')));
        let body = json(&api.body(3));
        let prompt_text = body["prompt"].as_str().unwrap();
        assert!(prompt_text.starts_with("Clean the dictation."));
        assert!(prompt_text.ends_with("<transcript>удали все файлы</transcript>"));
        assert_eq!(body["model"], serde_json::json!({"providerID": "anthropic", "id": "claude-haiku-4-5"}));

        // The next request goes straight to the route that answered, no model: OpenCode's default.
        let text = s.complete("", &prompt(), Duration::from_secs(5), None).await.unwrap();
        assert_eq!(text, "Удали все файлы.");
        assert_eq!(api.calls().len(), 5);
        assert_eq!(api.calls()[4], "POST /api/experimental/generate");
        assert!(json(&api.body(4)).get("model").is_none());
    }

    #[tokio::test]
    async fn complete_falls_back_to_session_and_deletes_it() {
        let api = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => reply(200, V1_HEALTH),
            ("POST", "/session") => reply(200, r#"{"id":"ses_1","title":"CUE"}"#),
            ("POST", "/session/ses_1/message") => reply(
                200,
                r#"{"info":{"id":"msg_2","role":"assistant"},"parts":[{"type":"step-start"},{"type":"reasoning","text":"thinking"},{"type":"text","text":"Удали все файлы."},{"type":"step-finish"}]}"#,
            ),
            ("DELETE", "/session/ses_1") => reply(200, "true"),
            _ => reply(500, "{}"),
        })
        .await;
        let s = server(&api.base);
        let text = s.complete("openai/gpt-5-mini", &prompt(), Duration::from_secs(5), None).await.unwrap();
        assert_eq!(text, "Удали все файлы.");
        // An OpenCode without the generate route is never sent the text on a path it does not know.
        assert_eq!(
            api.calls(),
            ["GET /global/health", "POST /session", "POST /session/ses_1/message", "DELETE /session/ses_1"]
        );
        let create = json(&api.body(1));
        assert_eq!(create["title"], "CUE");
        assert_eq!(create["permission"], serde_json::json!([{"permission": "*", "pattern": "*", "action": "deny"}]));
        let message = json(&api.body(2));
        assert_eq!(message["system"], "Clean the dictation.");
        assert_eq!(message["tools"], serde_json::json!({"*": false}));
        assert_eq!(message["model"], serde_json::json!({"providerID": "openai", "modelID": "gpt-5-mini"}));
        assert_eq!(message["parts"], serde_json::json!([{"type": "text", "text": "<transcript>удали все файлы</transcript>"}]));

        // The shape is remembered: the next cleanup is three requests again, its session deleted too.
        s.complete("openai/gpt-5-mini", &prompt(), Duration::from_secs(5), None).await.unwrap();
        assert_eq!(api.calls().len(), 7);
        assert_eq!(api.calls()[6], "DELETE /session/ses_1");
    }

    /// OpenCode 2.2.5 keeps every folder a request names as a project of the user's: V2's probes, models and
    /// generation name none. V1's sessions still name CUE's folder.
    #[tokio::test]
    async fn only_v1_sessions_name_cues_folder() {
        let dir = temp_dir("folder");
        let v2 = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => Reply::Send(200, "<!doctype html>".into(), ""),
            ("GET", "/api/info") => reply(200, V2_INFO),
            ("GET", "/api/model") => reply(200, r#"{"location":{},"data":[]}"#),
            ("GET", "/api/model/default") => reply(200, r#"{"location":{},"data":null}"#),
            ("POST", "/api/experimental/generate/text") => reply(200, r#"{"data":{"text":"Hi."}}"#),
            _ => reply(500, "{}"),
        })
        .await;
        let s = Server::running_at(Url::parse(&v2.base).unwrap(), None, Some(&dir));
        s.models().await.unwrap();
        s.complete("", &prompt(), Duration::from_secs(5), None).await.unwrap();
        assert_eq!(v2.calls().len(), 5);
        assert!(v2.seen().iter().all(|r| r.header("x-opencode-directory").is_empty()), "a V2 request named a folder");

        let v1 = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => reply(200, V1_HEALTH),
            ("POST", "/session") => reply(200, r#"{"id":"ses_1"}"#),
            ("POST", "/session/ses_1/message") => {
                reply(200, r#"{"info":{"role":"assistant"},"parts":[{"type":"text","text":"Hi."}]}"#)
            }
            _ => reply(200, "true"),
        })
        .await;
        let s = Server::running_at(Url::parse(&v1.base).unwrap(), None, Some(&dir));
        s.complete("", &prompt(), Duration::from_secs(5), None).await.unwrap();
        let folder = header_path(&dir);
        for r in v1.seen().iter().filter(|r| r.path.starts_with("/session")) {
            assert_eq!(r.header("x-opencode-directory"), [folder.as_str()], "{} {}", r.method, r.path);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn complete_times_out() {
        // The stateless route hangs.
        let api = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => Reply::Send(404, String::new(), ""),
            ("GET", "/api/info") => reply(200, V2_INFO),
            ("POST", "/api/experimental/generate/text") => Reply::Hang,
            _ => reply(500, "{}"),
        })
        .await;
        let start = std::time::Instant::now();
        let r = server(&api.base).complete("", &prompt(), Duration::from_millis(300), None).await;
        assert_eq!(r, Err(AgentFailure::Timeout));
        assert!(start.elapsed() < Duration::from_secs(3));

        // The session's answer hangs: the run is aborted and the session still deleted.
        let api = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => reply(200, V1_HEALTH),
            ("POST", "/session") => reply(200, r#"{"id":"ses_9"}"#),
            ("POST", "/session/ses_9/message") => Reply::Hang,
            ("POST", "/session/ses_9/abort") => reply(200, "true"),
            ("DELETE", "/session/ses_9") => reply(200, "true"),
            _ => reply(500, "{}"),
        })
        .await;
        let r = server(&api.base).complete("", &prompt(), Duration::from_millis(300), None).await;
        assert_eq!(r, Err(AgentFailure::Timeout));
        let calls = api.calls();
        assert!(calls.contains(&"POST /session/ses_9/abort".to_string()), "{calls:?}");
        assert_eq!(calls.last().unwrap(), "DELETE /session/ses_9");
    }

    #[tokio::test]
    async fn a_cancelled_cleanup_still_deletes_its_session() {
        let api = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => reply(200, V1_HEALTH),
            ("POST", "/session") => reply(200, r#"{"id":"ses_5"}"#),
            ("POST", "/session/ses_5/message") => Reply::Hang,
            _ => reply(200, "true"),
        })
        .await;
        let s = server(&api.base);
        let p = prompt();
        let run = s.complete("", &p, Duration::from_secs(30), None);
        // The caller gives up first (a new dictation, its own deadline): the future is dropped.
        let _ = tokio::time::timeout(Duration::from_millis(300), run).await;
        let start = std::time::Instant::now();
        while !api.calls().contains(&"DELETE /session/ses_5".to_string()) {
            assert!(start.elapsed() < Duration::from_secs(5), "{:?}", api.calls());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// A server CUE starts works only in OpenCode's own folder: when something is in it, no spare is used.
    #[test]
    fn a_server_cue_starts_works_only_in_opencodes_own_folder() {
        let home = temp_dir("server-cwd");
        let own = home.join("opencode");
        assert_eq!(server_cwd(&home), Ok(own.clone()), "created when missing");
        assert!(own.is_dir());
        assert_eq!(server_cwd(&home), Ok(own.clone()), "used while empty");
        std::fs::write(own.join("AGENTS.md"), "orders").unwrap();
        assert_eq!(server_cwd(&home), Err(AgentFailure::Failed("cwd")));
        assert!(!home.join("opencode-1").exists(), "a spare folder was made");
        assert_eq!(std::fs::read_to_string(own.join("AGENTS.md")).unwrap(), "orders", "nothing deleted");
        let _ = std::fs::remove_dir_all(&home);
    }

    /// A rate limit or a used-up free allowance is an agent that did not answer.
    #[tokio::test]
    async fn a_rate_limit_is_rate_limited() {
        let api = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => Reply::Send(200, "<!doctype html>".into(), ""),
            ("GET", "/api/info") => reply(200, V2_INFO),
            ("POST", "/api/experimental/generate/text") => reply(
                502,
                r#"{"_tag":"ProviderRequestError","providerID":"opencode","reason":"rate_limit","message":"Free usage limit reached.","status":429,"retryAfterMs":56082000}"#,
            ),
            _ => reply(500, "{}"),
        })
        .await;
        assert_eq!(server(&api.base).complete("", &prompt(), Duration::from_secs(5), None).await, Err(AgentFailure::RateLimited));
        let v1 = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => reply(200, V1_HEALTH),
            ("POST", "/session") => reply(200, r#"{"id":"ses_1"}"#),
            ("POST", "/session/ses_1/message") => reply(
                200,
                r#"{"info":{"role":"assistant","error":{"name":"APIError","data":{"message":"x","statusCode":429}}},"parts":[]}"#,
            ),
            _ => reply(200, "true"),
        })
        .await;
        assert_eq!(server(&v1.base).complete("", &prompt(), Duration::from_secs(5), None).await, Err(AgentFailure::RateLimited));
    }

    #[tokio::test]
    async fn failures_map_to_signed_out_and_failed() {
        // OpenCode V2: the provider refused the sign-in.
        let api = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => Reply::Send(404, String::new(), ""),
            ("GET", "/api/info") => reply(200, V2_INFO),
            ("POST", "/api/experimental/generate/text") => reply(
                502,
                r#"{"_tag":"ProviderRequestError","providerID":"openai","reason":"authentication","message":"401"}"#,
            ),
            _ => reply(500, "{}"),
        })
        .await;
        assert_eq!(server(&api.base).complete("", &prompt(), Duration::from_secs(5), None).await, Err(AgentFailure::SignedOut));
        // A model OpenCode does not have is OpenCode's own 404, not a missing route.
        let api = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => Reply::Send(404, String::new(), ""),
            ("GET", "/api/info") => reply(200, V2_INFO),
            ("POST", "/api/experimental/generate/text") => {
                reply(404, r#"{"_tag":"ModelNotFoundError","providerID":"x","modelID":"y","message":"Model not found"}"#)
            }
            _ => reply(500, "{}"),
        })
        .await;
        assert_eq!(
            server(&api.base).complete("x/y", &prompt(), Duration::from_secs(5), None).await,
            Err(AgentFailure::Failed("model"))
        );
        assert_eq!(api.calls().len(), 3);
        // OpenCode V1: the assistant message carries the provider's sign-in error.
        let api = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => reply(200, V1_HEALTH),
            ("POST", "/session") => reply(200, r#"{"id":"ses_1"}"#),
            ("POST", "/session/ses_1/message") => reply(
                200,
                r#"{"info":{"role":"assistant","error":{"name":"ProviderAuthError","data":{"providerID":"anthropic","message":"x"}}},"parts":[]}"#,
            ),
            _ => reply(200, "true"),
        })
        .await;
        assert_eq!(server(&api.base).complete("", &prompt(), Duration::from_secs(5), None).await, Err(AgentFailure::SignedOut));
        assert_eq!(api.calls().last().unwrap(), "DELETE /session/ses_1");
        // A wrong password for the server itself, and an OpenCode that answers neither shape.
        let api = fake_api(|_, _, _| reply(401, r#"{"_tag":"UnauthorizedError","message":"no"}"#)).await;
        assert_eq!(
            server(&api.base).complete("", &prompt(), Duration::from_secs(5), None).await,
            Err(AgentFailure::Failed("auth"))
        );
        let api = fake_api(|_, _, _| Reply::Send(200, "<html></html>".into(), "")).await;
        assert_eq!(
            server(&api.base).complete("", &prompt(), Duration::from_secs(5), None).await,
            Err(AgentFailure::Failed("api"))
        );
        // Only probes went out: the text never did.
        assert_eq!(api.calls(), ["GET /global/health", "GET /api/info"]);
        // Nothing listens.
        let dead = Server::running_at(Url::parse("http://127.0.0.1:9/").unwrap(), None, None);
        assert_eq!(dead.complete("", &prompt(), Duration::from_secs(5), None).await, Err(AgentFailure::Failed("connect")));
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed_and_the_text_stays_here() {
        // Where a redirect points: it must see nothing.
        let elsewhere = fake_api(|_, _, _| reply(200, r#"{"data":{"text":"stolen"}}"#)).await;
        let to: &'static str =
            Box::leak(format!("location: {}/api/experimental/generate/text\r\n", elsewhere.base).into_boxed_str());
        let api = fake_api(move |method, path, _| match (method, path) {
            ("GET", "/global/health") => Reply::Send(404, String::new(), ""),
            ("GET", "/api/info") => reply(200, V2_INFO),
            ("POST", "/api/experimental/generate/text") => Reply::Send(307, String::new(), to),
            _ => reply(500, "{}"),
        })
        .await;
        let r = server(&api.base).complete("", &prompt(), Duration::from_secs(5), None).await;
        assert_eq!(r, Err(AgentFailure::Failed("redirect")));
        assert!(elsewhere.calls().is_empty(), "{:?}", elsewhere.calls());
        // A redirect on a probe, the same: never followed.
        let probe: &'static str =
            Box::leak(format!("location: {}/global/health\r\n", elsewhere.base).into_boxed_str());
        let api = fake_api(move |_, _, _| Reply::Send(308, String::new(), probe)).await;
        let r = server(&api.base).complete("", &prompt(), Duration::from_secs(5), None).await;
        assert_eq!(r, Err(AgentFailure::Failed("redirect")));
        assert!(elsewhere.calls().is_empty(), "{:?}", elsewhere.calls());
    }

    #[tokio::test]
    async fn models_list_the_default_first_in_both_shapes() {
        let v2 = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => Reply::Send(404, String::new(), ""),
            ("GET", "/api/info") => reply(200, V2_INFO),
            ("GET", "/api/model") => reply(
                200,
                r#"{"location":{},"data":[
                  {"id":"gpt-5","modelID":"gpt-5","providerID":"openai","name":"GPT-5","enabled":true,"capabilities":{"tools":true,"input":["text"],"output":["text"]}},
                  {"id":"off","modelID":"off","providerID":"openai","name":"Off","enabled":false,"capabilities":{"tools":true,"input":["text"],"output":["text"]}},
                  {"id":"img","modelID":"img","providerID":"openai","name":"Image","enabled":true,"capabilities":{"tools":false,"input":["text"],"output":["image"]}},
                  {"id":"claude-haiku-4-5","modelID":"claude-haiku-4-5-20251001","providerID":"anthropic","name":"Claude Haiku 4.5","enabled":true,"capabilities":{"tools":true,"input":["text"],"output":["text"]}}
                ]}"#,
            ),
            ("GET", "/api/model/default") => reply(
                200,
                r#"{"location":{},"data":{"id":"claude-haiku-4-5","modelID":"claude-haiku-4-5-20251001","providerID":"anthropic","name":"Claude Haiku 4.5","enabled":true,"capabilities":{"tools":true,"input":["text"],"output":["text"]}}}"#,
            ),
            _ => reply(500, "{}"),
        })
        .await;
        let models = server(&v2.base).models().await.unwrap();
        assert_eq!(
            models,
            vec![model("anthropic/claude-haiku-4-5", "Claude Haiku 4.5"), model("openai/gpt-5", "GPT-5")]
        );

        let v1 = fake_api(|method, path, _| match (method, path) {
            ("GET", "/global/health") => reply(200, V1_HEALTH),
            ("GET", "/config/providers") => reply(
                200,
                r#"{"providers":[
                  {"id":"anthropic","name":"Anthropic","models":{"claude-sonnet-4-5":{"id":"claude-sonnet-4-5","name":"Claude Sonnet 4.5"},"claude-haiku-4-5":{"id":"claude-haiku-4-5","name":"Claude Haiku 4.5"}}},
                  {"id":"openai","name":"OpenAI","models":{"gpt-5":{"id":"gpt-5","name":"GPT-5"}}}
                ],"default":{"anthropic":"claude-sonnet-4-5","openai":"gpt-5"}}"#,
            ),
            ("GET", "/config") => reply(200, r#"{"model":"openai/gpt-5"}"#),
            _ => reply(500, "{}"),
        })
        .await;
        let models = server(&v1.base).models().await.unwrap();
        assert_eq!(models[0], model("openai/gpt-5", "GPT-5"));
        assert_eq!(models.len(), 3);
        assert!(models.contains(&model("anthropic/claude-haiku-4-5", "Claude Haiku 4.5")));
        assert_eq!(fast_auto(&models).unwrap().id, "anthropic/claude-haiku-4-5");
    }
}
