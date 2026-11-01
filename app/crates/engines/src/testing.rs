//! Engine test helpers: a fake HTTP/1.1 service on 127.0.0.1 and log capture —
//! no network and no new dependencies.
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

/// A request the fake service saw.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    /// Name in lower case, value without spaces at the edges; in arrival order.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// Connection number (0, 1, …, in accept order): the same number means the request came over the same connection.
    pub conn: usize,
}

impl Seen {
    /// All values of header `name` (name in lower case).
    pub fn header(&self, name: &str) -> Vec<&str> {
        self.headers.iter().filter(|(n, _)| n == name).map(|(_, v)| v.as_str()).collect()
    }
}

/// A reply of the fake service.
pub enum Reply {
    /// Status, body, extra header lines (`"retry-after: 0\r\n"` or empty).
    Send(u16, String, &'static str),
    /// Promise a body and drop the connection: a network failure mid-response.
    Cut,
    /// Close the connection without replying: a network failure before the response.
    Close,
    /// Never reply: the service hangs.
    Hang,
    /// A stream (SSE), like a real service over HTTP/1.1: status, then the body in `transfer-encoding: chunked`
    /// pieces — each piece a separate write, with a pause before each next one; after the same pause, the final
    /// empty chunk, and the connection stays open for the next request.
    Stream(u16, Vec<Vec<u8>>, Duration),
}

/// A plain reply: status and a JSON body.
pub fn reply(code: u16, body: &str) -> Reply {
    Reply::Send(code, body.to_owned(), "")
}

pub struct FakeApi {
    /// `http://127.0.0.1:<port>`, without a trailing slash.
    pub base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeApi {
    /// All requests, in order.
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    /// Method and path of each request, in order.
    pub fn calls(&self) -> Vec<String> {
        self.seen().iter().map(|r| format!("{} {}", r.method, r.path)).collect()
    }

    pub fn body(&self, i: usize) -> Vec<u8> {
        self.seen.lock().unwrap()[i].body.clone()
    }
}

/// Fake service (HTTP/1.1, connections are reused).
/// `route(method, path, how many such requests came before)` → reply.
pub async fn fake_api<F>(route: F) -> FakeApi
where
    F: Fn(&str, &str, usize) -> Reply + Send + Sync + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let (route, log) = (Arc::new(route), seen.clone());
    tokio::spawn(async move {
        let mut conn = 0;
        while let Ok((sock, _)) = listener.accept().await {
            tokio::spawn(serve(sock, conn, route.clone(), log.clone()));
            conn += 1;
        }
    });
    FakeApi { base, seen }
}

async fn serve<F>(sock: tokio::net::TcpStream, conn: usize, route: Arc<F>, seen: Arc<Mutex<Vec<Seen>>>)
where
    F: Fn(&str, &str, usize) -> Reply,
{
    let mut rd = BufReader::new(sock);
    loop {
        let mut line = String::new();
        if rd.read_line(&mut line).await.unwrap_or(0) == 0 {
            return; // the client closed the connection
        }
        let mut words = line.split_whitespace();
        let method = words.next().unwrap_or_default().to_owned();
        let path = words.next().unwrap_or_default().to_owned();
        let (mut len, mut headers) = (0, Vec::new());
        loop {
            let mut header = String::new();
            if rd.read_line(&mut header).await.unwrap_or(0) == 0 {
                return;
            }
            // An empty line ends the headers.
            let Some((name, value)) = header.trim_end().split_once(':') else { break };
            let (name, value) = (name.to_ascii_lowercase(), value.trim().to_owned());
            if name == "content-length" {
                len = value.parse().unwrap();
            }
            headers.push((name, value));
        }
        let mut body = vec![0; len];
        if rd.read_exact(&mut body).await.is_err() {
            return;
        }
        let nth = {
            let mut seen = seen.lock().unwrap();
            let nth = seen.iter().filter(|r| r.method == method && r.path == path).count();
            seen.push(Seen { method: method.clone(), path: path.clone(), headers, body, conn });
            nth
        };
        let resp = match route(&method, &path, nth) {
            Reply::Send(code, body, extra) => format!(
                "HTTP/1.1 {code} Fake\r\ncontent-type: application/json\r\ncontent-length: {}\r\n{extra}\r\n{body}",
                body.len()
            ),
            Reply::Cut => {
                let _ = rd.get_mut().write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 100\r\n\r\n{\"id\"").await;
                return; // the connection closes mid-word
            }
            Reply::Close => return,
            Reply::Hang => return std::future::pending().await,
            Reply::Stream(code, pieces, gap) => {
                // No Nagle delay: a piece goes out at once instead of waiting for the previous one's ACK.
                let _ = rd.get_ref().set_nodelay(true);
                let head = format!("HTTP/1.1 {code} Fake\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n");
                let sock = rd.get_mut();
                if sock.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                for (i, piece) in pieces.iter().enumerate() {
                    if i > 0 {
                        tokio::time::sleep(gap).await;
                    }
                    let frame = [format!("{:x}\r\n", piece.len()).as_bytes(), piece, b"\r\n"].concat();
                    if sock.write_all(&frame).await.is_err() || sock.flush().await.is_err() {
                        return;
                    }
                }
                tokio::time::sleep(gap).await;
                if sock.write_all(b"0\r\n\r\n").await.is_err() {
                    return;
                }
                continue;
            }
        };
        if rd.get_mut().write_all(resp.as_bytes()).await.is_err() {
            return;
        }
    }
}

/// Log capture: `tracing` events of this thread as lines "LEVEL target: message fields…",
/// while the `DefaultGuard` lives. A `#[tokio::test]` test (single-threaded runtime) also sees its tasks' events.
pub fn capture_logs() -> (Arc<Mutex<Vec<String>>>, tracing::subscriber::DefaultGuard) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let guard = tracing::subscriber::set_default(Capture(lines.clone()));
    (lines, guard)
}

struct Capture(Arc<Mutex<Vec<String>>>);

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        struct Line(String);
        impl tracing::field::Visit for Line {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                let _ = match field.name() {
                    "message" => write!(self.0, " {value:?}"),
                    name => write!(self.0, " {name}={value:?}"),
                };
            }
        }
        let meta = event.metadata();
        let mut line = Line(format!("{} {}:", meta.level(), meta.target()));
        event.record(&mut line);
        self.0.lock().unwrap().push(line.0);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}
