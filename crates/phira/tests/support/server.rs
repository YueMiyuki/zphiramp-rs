use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;

const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEAD: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub at: Instant,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub enum Reply {
    Answer {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    Drop,
    Hang,
    After(Duration, Box<Reply>),
}

impl Reply {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self::status(200, body)
    }

    pub fn json(body: &str) -> Self {
        Self::Answer {
            status: 200,
            headers: vec![("Content-Type".into(), "application/json".into())],
            body: body.as_bytes().to_vec(),
        }
    }

    pub fn status(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self::Answer {
            status,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn after(delay: Duration, reply: Reply) -> Self {
        Self::After(delay, Box::new(reply))
    }

    pub fn redirect(status: u16, location: &str) -> Self {
        Self::Answer {
            status,
            headers: vec![("Location".into(), location.into())],
            body: Vec::new(),
        }
    }

    pub fn ranged(request: &Request, file: &[u8]) -> Self {
        let Some(range) = request.header("range") else {
            return Self::ok(file);
        };
        let spec = range.strip_prefix("bytes=").expect("bytes range");
        let (from, to) = spec.split_once('-').expect("a-b range");
        let len = file.len();
        let (start, end) = if from.is_empty() {
            let n: usize = to.parse().expect("suffix length");
            (len.saturating_sub(n), len.saturating_sub(1))
        } else {
            let start: usize = from.parse().expect("start");
            let end = if to.is_empty() {
                len.saturating_sub(1)
            } else {
                to.parse::<usize>().expect("end").min(len.saturating_sub(1))
            };
            (start, end)
        };
        if start >= len {
            return Self::Answer {
                status: 416,
                headers: vec![("Content-Range".into(), format!("bytes */{len}"))],
                body: Vec::new(),
            };
        }
        Self::Answer {
            status: 206,
            headers: vec![("Content-Range".into(), format!("bytes {start}-{end}/{len}"))],
            body: file[start..=end].to_vec(),
        }
    }
}

type Handler = dyn Fn(&Request, usize) -> Reply + Send + Sync;

pub struct Server {
    pub addr: SocketAddr,
    requests: Arc<Mutex<Vec<Request>>>,
    task: JoinHandle<()>,
}

impl Server {
    pub async fn start<F>(handler: F) -> Self
    where
        F: Fn(&Request, usize) -> Reply + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind 127.0.0.1");
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let log = requests.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, _)) = accepted else { break };
                        connections.spawn(serve(stream, handler.clone(), log.clone()));
                    }
                    Some(_) = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Self {
            addr,
            requests,
            task,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(mut stream: TcpStream, handler: Arc<Handler>, log: Arc<Mutex<Vec<Request>>>) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    let index = {
        let mut log = log.lock().unwrap();
        log.push(request.clone());
        log.len() - 1
    };
    let mut reply = handler(&request, index);
    while let Reply::After(delay, inner) = reply {
        tokio::time::sleep(delay).await;
        reply = *inner;
    }
    match reply {
        Reply::Drop | Reply::After(..) => {}
        Reply::Hang => tokio::time::sleep(Duration::from_secs(60)).await,
        Reply::Answer {
            status,
            headers,
            body,
        } => {
            let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
            head.push_str("Connection: close\r\n");
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            head.push_str("\r\n");
            let mut out = head.into_bytes();
            if request.method != "HEAD" {
                out.extend_from_slice(&body);
            }
            let _ = timeout(IO_TIMEOUT, stream.write_all(&out)).await;
            let _ = timeout(IO_TIMEOUT, stream.shutdown()).await;
        }
    }
}

async fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(position) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break position;
        }
        if buffer.len() > MAX_HEAD {
            return None;
        }
        let n = timeout(IO_TIMEOUT, stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            return None;
        }
        buffer.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(n, _)| n == "content-length")
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buffer[head_end + 4..].to_vec();
    while body.len() < length {
        let n = timeout(IO_TIMEOUT, stream.read(&mut chunk))
            .await
            .ok()?
            .ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(length);
    Some(Request {
        method,
        target,
        headers,
        body,
        at: Instant::now(),
    })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        307 => "Temporary Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Status",
    }
}
