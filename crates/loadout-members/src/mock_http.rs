//! A tiny loopback HTTP server for tests (feature `test-support`). Serves
//! fixed JSON responses by path and records each request's path and
//! `Authorization` and `Accept` headers. Loopback only — tests never use the
//! network.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub accept: Option<String>,
    pub body: String,
}

pub struct MockServer {
    addr: String,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<Request>>>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    /// Starts a server answering `routes` (path → (status, body)); other
    /// paths get 404.
    pub fn start(routes: BTreeMap<String, (u16, String)>) -> Self {
        Self::start_raw(
            routes
                .into_iter()
                .map(|(k, (s, b))| (k, (s, b.into_bytes())))
                .collect(),
        )
    }

    /// Like [`MockServer::start`] with raw bodies (e.g. archives).
    pub fn start_raw(routes: BTreeMap<String, (u16, Vec<u8>)>) -> Self {
        Self::start_with(|_| routes)
    }

    /// Builds the routes from the server's own base URL (for responses that
    /// link back to it).
    pub fn start_with(make: impl FnOnce(&str) -> BTreeMap<String, (u16, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        listener.set_nonblocking(true).unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let routes = make(&addr);
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (stop2, reqs2) = (stop.clone(), requests.clone());
        let handle = std::thread::spawn(move || {
            while !stop2.load(Ordering::Relaxed) {
                let Ok((stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() {
                    continue;
                }
                let method = line.split_whitespace().next().unwrap_or("").to_owned();
                let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                let mut authorization = None;
                let mut accept = None;
                let mut length = 0usize;
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                    if let Some((k, v)) = h.split_once(':') {
                        if k.eq_ignore_ascii_case("authorization") {
                            authorization = Some(v.trim().to_owned());
                        } else if k.eq_ignore_ascii_case("accept") {
                            accept = Some(v.trim().to_owned());
                        } else if k.eq_ignore_ascii_case("content-length") {
                            length = v.trim().parse().unwrap_or(0);
                        }
                    }
                }
                let mut body = vec![0u8; length];
                let _ = std::io::Read::read_exact(&mut reader, &mut body);
                reqs2.lock().unwrap().push(Request {
                    method,
                    path: path.clone(),
                    authorization,
                    accept,
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
                let (status, body) = routes
                    .get(&path)
                    .cloned()
                    .unwrap_or((404, br#"{"message":"Not Found"}"#.to_vec()));
                let mut stream = stream;
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(&body);
            }
        });
        MockServer {
            addr,
            stop,
            requests,
            handle: Some(handle),
        }
    }

    /// A GitHub API mock where `login` belongs to `teams` (`org/team`).
    pub fn github(login: &str, teams: &[&str]) -> Self {
        let mut routes = BTreeMap::new();
        routes.insert(
            "/user".to_owned(),
            (200, format!(r#"{{"login":"{login}"}}"#)),
        );
        for t in teams {
            let (org, team) = t.split_once('/').expect("org/team");
            routes.insert(
                format!("/orgs/{org}/teams/{team}/memberships/{login}"),
                (200, r#"{"state":"active","role":"member"}"#.to_owned()),
            );
        }
        Self::start(routes)
    }

    pub fn url(&self) -> &str {
        &self.addr
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
