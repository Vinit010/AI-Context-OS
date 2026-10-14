//! A local fake HTTP server for the contract tests.
//!
//! `docs/AI_PROVIDER_SPEC.md` §11 requires the contract tests to pass against a local fake server
//! with no network, so this binds an ephemeral `127.0.0.1` port and serves canned responses over
//! plain HTTP. It is `std`-only on purpose: the tests need a server, not a web framework.

#![allow(dead_code)]

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// One canned response: a status, headers, a body, and an optional delay.
#[derive(Clone, Debug)]
pub(crate) struct Canned {
    /// The HTTP status to answer with.
    pub status: u16,
    /// Extra response headers, as `(name, value)`.
    pub headers: Vec<(String, String)>,
    /// The response body.
    pub body: String,
    /// A pause before writing, used to exercise a timeout.
    pub delay: Option<Duration>,
}

impl Canned {
    /// A response with a body and no extra headers.
    #[must_use]
    pub(crate) fn new(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            delay: None,
        }
    }

    /// A JSON response.
    ///
    /// # Panics
    ///
    /// Panics if `value` does not serialise, which for a `Value` cannot happen.
    #[must_use]
    pub(crate) fn json(status: u16, value: &serde_json::Value) -> Self {
        let mut canned = Canned::new(
            status,
            serde_json::to_string(value).expect("a `serde_json::Value` always serialises"),
        );
        canned
            .headers
            .push(("content-type".to_owned(), "application/json".to_owned()));
        canned
    }

    /// Adds a response header.
    #[must_use]
    pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// Delays the response, to exercise a timeout.
    #[must_use]
    pub(crate) fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = Some(delay);
        self
    }
}

/// A request the server received, recorded so a test can assert on the wire.
#[derive(Clone, Debug, Default)]
pub(crate) struct RecordedRequest {
    /// The HTTP method.
    pub method: String,
    /// The request target, including any query.
    pub path: String,
    /// The headers, with lower-case names.
    pub headers: Vec<(String, String)>,
    /// The body, decoded as UTF-8.
    pub body: String,
}

impl RecordedRequest {
    /// The first value of `name`, case-insensitively.
    #[must_use]
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(key, _)| key == &name)
            .map(|(_, value)| value.as_str())
    }

    /// The body, parsed as JSON.
    ///
    /// # Panics
    ///
    /// Panics when the body is not JSON, which a test asserting on it is allowed to do.
    #[must_use]
    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("the recorded body is JSON")
    }
}

/// A fake server on an ephemeral localhost port.
pub(crate) struct FakeServer {
    address: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl FakeServer {
    /// Starts a server that answers with `responses` in order, then repeats the last one.
    ///
    /// # Panics
    ///
    /// Panics when the ephemeral port cannot be bound.
    #[must_use]
    pub(crate) fn start(responses: Vec<Canned>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
        let address = listener.local_addr().expect("a bound listener has an address");
        listener.set_nonblocking(true).expect("set non-blocking");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let responses = Arc::new(responses);
        let next = Arc::new(AtomicUsize::new(0));
        let handle = {
            let requests = Arc::clone(&requests);
            let stop = Arc::clone(&stop);
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let requests = Arc::clone(&requests);
                            let responses = Arc::clone(&responses);
                            let next = Arc::clone(&next);
                            thread::spawn(move || serve(stream, &requests, &responses, &next));
                        }
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Self {
            address,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    /// The root URL, `http://127.0.0.1:<port>`.
    #[must_use]
    pub(crate) fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// A snapshot of the requests received so far.
    ///
    /// # Panics
    ///
    /// Panics only if a handler panicked while holding the lock, which would already have failed
    /// the test.
    #[must_use]
    pub(crate) fn requests(&self) -> Vec<RecordedRequest> {
        self.requests
            .lock()
            .expect("the request log is not poisoned")
            .clone()
    }

    /// How many requests have been received.
    #[must_use]
    pub(crate) fn request_count(&self) -> usize {
        self.requests
            .lock()
            .expect("the request log is not poisoned")
            .len()
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Serves requests on one connection until the client closes it, then writes each canned response.
///
/// HTTP/1.1 is a persistent protocol, so a connection may carry several requests — which it does
/// here, because a retry reuses the pooled connection rather than racing a fresh one.
fn serve(
    stream: TcpStream,
    requests: &Arc<Mutex<Vec<RecordedRequest>>>,
    responses: &[Canned],
    next: &AtomicUsize,
) {
    let mut stream = stream;
    // A socket accepted by a non-blocking listener can inherit that mode on Windows, which would
    // make the first read return `WouldBlock` and close the connection. Blocking is the intent.
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    loop {
        let Some(request) = read_request(&mut stream) else {
            return;
        };
        requests
            .lock()
            .expect("the request log is not poisoned")
            .push(request);
        let index = next.fetch_add(1, Ordering::SeqCst);
        let response = responses
            .get(index)
            .or_else(|| responses.last())
            .cloned()
            .unwrap_or_else(|| Canned::new(500, "no canned response"));
        write_response(&mut stream, &response);
    }
}

/// Reads one HTTP/1.1 request, headers and body.
fn read_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        if let Some(position) = find(&buffer, b"\r\n\r\n") {
            break position + 4;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    };
    let head = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split(' ');
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_owned();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((name, value));
        }
    }
    let body_start = header_end;
    let body_end = body_start + content_length;
    while buffer.len() < body_end {
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    let body_end = body_end.min(buffer.len());
    let body = String::from_utf8_lossy(&buffer[body_start..body_end]).into_owned();
    Some(RecordedRequest {
        method,
        path,
        headers,
        body,
    })
}

/// Writes a canned response on a persistent connection.
fn write_response(stream: &mut TcpStream, response: &Canned) {
    if let Some(delay) = response.delay {
        thread::sleep(delay);
    }
    let mut head = format!("HTTP/1.1 {} {}\r\n", response.status, reason(response.status));
    head.push_str(&format!("content-length: {}\r\n", response.body.len()));
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(response.body.as_bytes());
    let _ = stream.flush();
}

/// The first index of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// A reason phrase for a status, which HTTP/1.1 permits to be generic.
fn reason(status: u16) -> &'static str {
    match status {
        200..=299 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        500..=599 => "Server Error",
        _ => "Status",
    }
}
