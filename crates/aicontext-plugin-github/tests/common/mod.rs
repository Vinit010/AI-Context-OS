//! A tiny loopback HTTP server for the plugin tests.
//!
//! The tests must not reach GitHub. This server accepts one request at a time, records it, and
//! answers from a closure. It is deliberately plain `std::net` code: no test-only HTTP dependency,
//! no TLS, and no behaviour the production client could accidentally rely on. Every response is
//! deterministic and every request is inspectable.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// One request the mock received.
#[derive(Clone, Debug)]
pub(crate) struct MockRequest {
    /// The HTTP method.
    pub(crate) method: String,
    /// The decoded path, without the query string.
    pub(crate) path: String,
    /// The decoded query parameters.
    pub(crate) query: BTreeMap<String, String>,
    /// The lower-case header names and their values.
    pub(crate) headers: BTreeMap<String, String>,
}

/// One response the mock sends.
#[derive(Clone, Debug)]
pub(crate) struct MockResponse {
    /// The HTTP status.
    pub(crate) status: u16,
    /// Extra response headers.
    pub(crate) headers: Vec<(String, String)>,
    /// The response body.
    pub(crate) body: String,
}

impl MockResponse {
    /// A 200 response with a JSON body.
    #[must_use]
    pub(crate) fn json(value: &serde_json::Value) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: value.to_string(),
        }
    }

    /// A response with an explicit status and body.
    #[must_use]
    pub(crate) fn status(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    /// Add one response header.
    #[must_use]
    pub(crate) fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }
}

/// A running mock server.
pub(crate) struct MockServer {
    /// The base URL, `http://127.0.0.1:<port>`.
    pub(crate) base: String,
    /// Every request the server has received.
    pub(crate) requests: Arc<Mutex<Vec<MockRequest>>>,
    stop: Arc<AtomicBool>,
}

impl MockServer {
    /// Start a server that answers every request with `handler`.
    pub(crate) fn start<F>(handler: F) -> Self
    where
        F: Fn(&MockRequest) -> MockResponse + Send + Sync + 'static,
    {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("the mock server binds a loopback port");
        let address = listener
            .local_addr()
            .expect("the mock server has a local address");
        listener
            .set_nonblocking(true)
            .expect("the mock listener is non-blocking");

        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let handler = Arc::new(handler);
        let thread_requests = Arc::clone(&requests);
        let thread_stop = Arc::clone(&stop);

        thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // Windows inherits the listener's non-blocking mode on `accept`, so the
                        // connection must be switched back before the request is read.
                        stream
                            .set_nonblocking(false)
                            .expect("the mock connection is blocking");
                        let request = read_request(&stream);
                        thread_requests
                            .lock()
                            .expect("the mock request log is not poisoned")
                            .push(request.clone());
                        let response = handler(&request);
                        write_response(stream, response);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            base: format!("http://{address}"),
            requests,
            stop,
        }
    }

    /// The requests received so far.
    #[must_use]
    pub(crate) fn requests(&self) -> Vec<MockRequest> {
        self.requests
            .lock()
            .expect("the mock request log is not poisoned")
            .clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn read_request(stream: &TcpStream) -> MockRequest {
    let reader_stream = stream.try_clone().expect("the mock can clone its stream");
    let mut reader = BufReader::new(reader_stream);

    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .expect("the mock reads the request line");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let target = parts.next().unwrap_or("/");
    let url = reqwest::Url::parse(&format!("http://localhost{target}"))
        .expect("the mock request target is a URL");
    let path = url.path().to_owned();
    let query = url
        .query_pairs()
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .expect("the mock reads a header line");
        if line == "\r\n" || line == "\n" || line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
        }
    }

    MockRequest {
        method,
        path,
        query,
        headers,
    }
}

fn write_response(mut stream: TcpStream, response: MockResponse) {
    let reason = match response.status {
        200 => "OK",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        _ => "Status",
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: \
         close\r\n",
        response.status,
        reason,
        response.body.len()
    );
    for (name, value) in response.headers {
        head.push_str(&name);
        head.push_str(": ");
        head.push_str(&value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");

    stream
        .write_all(head.as_bytes())
        .expect("the mock writes the response head");
    stream
        .write_all(response.body.as_bytes())
        .expect("the mock writes the response body");
    stream.flush().expect("the mock flushes the response");
}
