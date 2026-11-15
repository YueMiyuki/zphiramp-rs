// reqwest always adds `Accept: */*`; the range requests must carry only `User-Agent` and
// `Range`, so this module speaks hyper directly.
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Empty};
use hyper::body::{Bytes, Incoming};
use hyper::header::{CONTENT_LENGTH, CONTENT_RANGE, LOCATION, RANGE, USER_AGENT};
use hyper::{Method, Request, Uri};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use reqwest::Url;

use crate::PhiraError;
use crate::error::Kind;

const USER_AGENT_VALUE: &str = "JPhira/1";
// Four redirects are followed; the fifth 3xx answer is returned as is.
const MAX_REDIRECTS: usize = 4;

pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) content_range_total: Option<u64>,
    pub(crate) content_length: Option<u64>,
    pub(crate) body: Incoming,
}

pub(crate) struct ProbeHttp {
    client: Client<HttpsConnector<HttpConnector>, Empty<Bytes>>,
}

impl ProbeHttp {
    pub(crate) fn new(connect_timeout: Duration) -> Result<Self, PhiraError> {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let verifier =
            rustls_platform_verifier::Verifier::new(provider.clone()).map_err(tls_setup)?;
        Self::build(connect_timeout, provider, Arc::new(verifier))
    }

    #[cfg(test)]
    pub(crate) fn with_test_roots(
        connect_timeout: Duration,
        roots: Vec<rustls::pki_types::CertificateDer<'static>>,
    ) -> Result<Self, PhiraError> {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut store = rustls::RootCertStore::empty();
        for root in roots {
            store.add(root).map_err(tls_setup)?;
        }
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::new(store),
            provider.clone(),
        )
        .build()
        .map_err(|error| {
            PhiraError::new(Kind::InvalidRequest(format!("TLS setup failed: {error}")))
        })?;
        Self::build(connect_timeout, provider, verifier)
    }

    fn build(
        connect_timeout: Duration,
        provider: Arc<rustls::crypto::CryptoProvider>,
        verifier: Arc<dyn rustls::client::danger::ServerCertVerifier>,
    ) -> Result<Self, PhiraError> {
        let tls = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(tls_setup)?
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();

        let mut http = HttpConnector::new();
        http.enforce_http(false);
        http.set_connect_timeout(Some(connect_timeout));
        http.set_nodelay(true);
        let connector = HttpsConnectorBuilder::new()
            .with_tls_config(tls)
            .https_or_http()
            .enable_http1()
            .enable_http2()
            .wrap_connector(http);
        Ok(Self {
            client: Client::builder(TokioExecutor::new()).build(connector),
        })
    }

    pub(crate) async fn get(&self, url: &Url, range: &str) -> Result<Response, PhiraError> {
        let mut url = url.clone();
        let mut followed = 0;
        loop {
            let uri = Uri::try_from(url.as_str())
                .map_err(|error| PhiraError::new(Kind::InvalidUrl(format!("{url}: {error}"))))?;
            let request = Request::builder()
                .method(Method::GET)
                .uri(uri)
                .header(USER_AGENT, USER_AGENT_VALUE)
                .header(RANGE, range)
                .body(Empty::<Bytes>::new())
                .map_err(|error| PhiraError::new(Kind::InvalidRequest(error.to_string())))?;
            let response = self.client.request(request).await.map_err(|error| {
                let headline = if error.is_connect() {
                    "HTTP connect failed"
                } else {
                    "HTTP request failed"
                };
                PhiraError::transport_with_cause(headline, Some(&error))
            })?;
            let status = response.status().as_u16();
            let next = (followed < MAX_REDIRECTS)
                .then(|| redirect_target(&url, status, response.headers().get(LOCATION)))
                .flatten();
            if let Some(next) = next {
                url = next;
                followed += 1;
                continue;
            }
            let headers = response.headers();
            let content_range_total = headers
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(content_range_total);
            let content_length = headers
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.trim().parse().ok());
            return Ok(Response {
                status,
                content_range_total,
                content_length,
                body: response.into_body(),
            });
        }
    }
}

fn tls_setup(error: rustls::Error) -> PhiraError {
    PhiraError::new(Kind::InvalidRequest(format!("TLS setup failed: {error}")))
}

fn redirect_target(
    from: &Url,
    status: u16,
    location: Option<&hyper::header::HeaderValue>,
) -> Option<Url> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let next = from.join(location?.to_str().ok()?).ok()?;
    let downgrade = from.scheme() == "https" && next.scheme() == "http";
    (matches!(next.scheme(), "http" | "https") && !downgrade).then_some(next)
}

fn content_range_total(value: &str) -> Option<u64> {
    value.rsplit_once('/')?.1.trim().parse().ok()
}

pub(crate) async fn next_chunk(body: &mut Incoming) -> Result<Option<Bytes>, PhiraError> {
    loop {
        match body.frame().await {
            None => return Ok(None),
            Some(Err(error)) => {
                return Err(PhiraError::transport_with_cause(
                    "HTTP request failed",
                    Some(&error),
                ));
            }
            Some(Ok(frame)) => {
                if let Ok(data) = frame.into_data() {
                    return Ok(Some(data));
                }
            }
        }
    }
}

pub(crate) fn timed_out() -> PhiraError {
    PhiraError::transport_with_cause("HTTP request timed out", None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::HeaderValue;

    #[test]
    fn content_range_total_of_the_answer() {
        assert_eq!(content_range_total("bytes 0-99/12345"), Some(12345));
        assert_eq!(content_range_total("bytes 0-99/*"), None);
        assert_eq!(content_range_total("garbage"), None);
        assert_eq!(content_range_total("bytes */777"), Some(777));
    }

    #[test]
    fn redirects_follow_the_jdk_rules() {
        let http = Url::parse("http://a.example/x/y").unwrap();
        let https = Url::parse("https://a.example/x/y").unwrap();
        let location = |v: &'static str| Some(HeaderValue::from_static(v));
        for status in [301u16, 302, 303, 307, 308] {
            assert_eq!(
                redirect_target(&http, status, location("/z?sign=1").as_ref())
                    .unwrap()
                    .as_str(),
                "http://a.example/z?sign=1"
            );
        }
        for status in [200u16, 206, 300, 304, 305, 404, 500] {
            assert!(
                redirect_target(&http, status, location("/z").as_ref()).is_none(),
                "{status}"
            );
        }
        assert_eq!(
            redirect_target(&https, 302, location("z").as_ref())
                .unwrap()
                .as_str(),
            "https://a.example/x/z"
        );
        assert_eq!(
            redirect_target(&https, 302, location("//b.example/q").as_ref())
                .unwrap()
                .as_str(),
            "https://b.example/q"
        );
        assert_eq!(
            redirect_target(&http, 302, location("https://c.example/").as_ref())
                .unwrap()
                .as_str(),
            "https://c.example/"
        );
        assert!(redirect_target(&https, 302, location("http://c.example/").as_ref()).is_none());
        assert!(redirect_target(&http, 302, None).is_none());
        assert!(redirect_target(&http, 302, location("ftp://x/").as_ref()).is_none());
        assert!(redirect_target(&http, 302, location("http://[bad").as_ref()).is_none());
    }

    use std::convert::Infallible;
    use std::sync::Mutex;

    use http_body_util::Full;
    use hyper::service::service_fn;
    use hyper::{Response as HyperResponse, Version};
    use hyper_util::rt::TokioIo;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_rustls::TlsAcceptor;

    const CA: &[u8] = include_bytes!("../../tests/fixtures/tls/ca.der");
    const LEAF: &[u8] = include_bytes!("../../tests/fixtures/tls/leaf.der");
    const LEAF_KEY: &[u8] = include_bytes!("../../tests/fixtures/tls/leaf.key.der");

    const BOUND: Duration = Duration::from_secs(20);

    fn acceptor(alpn: &[&[u8]]) -> TlsAcceptor {
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(LEAF.to_vec())],
                PrivateKeyDer::from(PrivatePkcs8KeyDer::from(LEAF_KEY.to_vec())),
            )
            .unwrap();
        config.alpn_protocols = alpn.iter().map(|p| p.to_vec()).collect();
        TlsAcceptor::from(Arc::new(config))
    }

    fn client() -> ProbeHttp {
        ProbeHttp::with_test_roots(
            Duration::from_secs(5),
            vec![CertificateDer::from(CA.to_vec())],
        )
        .unwrap()
    }

    async fn https_h1(
        answer: impl Fn(&str) -> String + Send + Sync + 'static,
    ) -> (u16, Arc<Mutex<Vec<String>>>, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let heads = Arc::new(Mutex::new(Vec::new()));
        let recorded = heads.clone();
        let acceptor = acceptor(&[b"http/1.1"]);
        let answer = Arc::new(answer);
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let (acceptor, recorded, answer) =
                    (acceptor.clone(), recorded.clone(), answer.clone());
                tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let mut head = Vec::new();
                    let mut chunk = [0u8; 2048];
                    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                        match tokio::time::timeout(BOUND, tls.read(&mut chunk)).await {
                            Ok(Ok(n)) if n > 0 => head.extend_from_slice(&chunk[..n]),
                            _ => return,
                        }
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let response = answer(&head);
                    recorded.lock().unwrap().push(head);
                    let _ = tls.write_all(response.as_bytes()).await;
                    let _ = tls.shutdown().await;
                });
            }
        });
        (port, heads, task)
    }

    fn header_names(head: &str) -> Vec<String> {
        let mut names: Vec<String> = head
            .split("\r\n")
            .skip(1)
            .filter_map(|l| l.split_once(':'))
            .map(|(n, _)| n.to_ascii_lowercase())
            .collect();
        names.sort();
        names
    }

    async fn read_all(mut response: Response) -> (u16, Option<u64>, Vec<u8>) {
        let mut body = Vec::new();
        while let Some(chunk) = next_chunk(&mut response.body).await.unwrap() {
            body.extend_from_slice(&chunk);
        }
        (response.status, response.content_range_total, body)
    }

    #[tokio::test]
    async fn https_http1_request_has_only_the_java_headers() {
        let (port, heads, server) = https_h1(|_| {
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-4/1000\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".into()
        })
        .await;
        let url = Url::parse(&format!("https://127.0.0.1:{port}/files/x?sign=abc")).unwrap();
        let response = tokio::time::timeout(BOUND, client().get(&url, "bytes=0-4"))
            .await
            .expect("bounded")
            .unwrap();
        let (status, total, body) = tokio::time::timeout(BOUND, read_all(response))
            .await
            .unwrap();
        assert_eq!(
            (status, total, body.as_slice()),
            (206, Some(1000), &b"hello"[..])
        );
        let head = heads.lock().unwrap()[0].clone();
        assert!(
            head.starts_with("GET /files/x?sign=abc HTTP/1.1\r\n"),
            "{head}"
        );
        assert_eq!(
            header_names(&head),
            ["host", "range", "user-agent"],
            "{head}"
        );
        assert!(
            head.contains("user-agent: JPhira/1\r\n") && head.contains("range: bytes=0-4\r\n"),
            "{head}"
        );
        server.abort();
    }

    #[tokio::test]
    async fn https_http2_is_negotiated_and_sends_only_the_java_headers() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        type Seen = Vec<(Version, Vec<(String, String)>)>;
        let seen: Arc<Mutex<Seen>> = Arc::default();
        let recorded = seen.clone();
        let acceptor = acceptor(&[b"h2", b"http/1.1"]);
        let server = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    return;
                };
                let (acceptor, recorded) = (acceptor.clone(), recorded.clone());
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let service =
                        service_fn(move |request: hyper::Request<hyper::body::Incoming>| {
                            let headers = request
                                .headers()
                                .iter()
                                .map(|(n, v)| {
                                    (n.as_str().to_owned(), v.to_str().unwrap().to_owned())
                                })
                                .collect();
                            recorded.lock().unwrap().push((request.version(), headers));
                            async {
                                Ok::<_, Infallible>(
                                    HyperResponse::builder()
                                        .status(206)
                                        .header("content-range", "bytes 0-4/777")
                                        .body(Full::new(Bytes::from_static(b"hello")))
                                        .unwrap(),
                                )
                            }
                        });
                    let _ = hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                        .serve_connection(TokioIo::new(tls), service)
                        .await;
                });
            }
        });
        let url = Url::parse(&format!("https://127.0.0.1:{port}/files/x")).unwrap();
        let probe = client();
        let response = tokio::time::timeout(BOUND, probe.get(&url, "bytes=0-4"))
            .await
            .unwrap()
            .unwrap();
        let (status, total, body) = tokio::time::timeout(BOUND, read_all(response))
            .await
            .unwrap();
        assert_eq!(
            (status, total, body.as_slice()),
            (206, Some(777), &b"hello"[..])
        );
        let response = tokio::time::timeout(BOUND, probe.get(&url, "bytes=-5"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read_all(response).await.0, 206);
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        for (version, headers) in &seen {
            assert_eq!(*version, Version::HTTP_2);
            let mut names: Vec<&str> = headers.iter().map(|(n, _)| n.as_str()).collect();
            names.sort_unstable();
            assert_eq!(names, ["range", "user-agent"], "{headers:?}");
        }
        assert_eq!(
            seen[1].1.iter().find(|(n, _)| n == "range").unwrap().1,
            "bytes=-5"
        );
        server.abort();
    }

    #[tokio::test]
    async fn an_untrusted_certificate_is_a_transport_error() {
        let (port, heads, server) =
            https_h1(|_| "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".into()).await;
        let url = Url::parse(&format!("https://127.0.0.1:{port}/")).unwrap();
        let probe = ProbeHttp::new(Duration::from_secs(5)).unwrap();
        let error = match tokio::time::timeout(BOUND, probe.get(&url, "bytes=0-4"))
            .await
            .expect("bounded")
        {
            Ok(_) => panic!("the self-signed test CA must not be trusted by default"),
            Err(error) => error,
        };
        assert!(error.is_transport(), "{error}");
        assert!(
            heads.lock().unwrap().is_empty(),
            "no request was sent over the failed handshake"
        );
        server.abort();
    }

    #[tokio::test]
    async fn redirects_over_https() {
        let (port, heads, server) = https_h1(|head| {
            let target = head.split(' ').nth(1).unwrap_or("");
            match target {
                "/start" => "HTTP/1.1 303 See Other\r\nLocation: /next\r\nContent-Length: 0\r\n\r\n".to_owned(),
                "/next" => "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-1/9\r\nContent-Length: 2\r\n\r\nok".to_owned(),
                _ => "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/plain\r\nContent-Length: 0\r\n\r\n".to_owned(),
            }
        })
        .await;
        let probe = client();
        let followed = Url::parse(&format!("https://127.0.0.1:{port}/start")).unwrap();
        let response = tokio::time::timeout(BOUND, probe.get(&followed, "bytes=0-1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read_all(response).await, (206, Some(9), b"ok".to_vec()));
        {
            let heads = heads.lock().unwrap();
            assert_eq!(heads.len(), 2);
            assert_eq!(header_names(&heads[1]), ["host", "range", "user-agent"]);
            assert!(heads[1].contains("range: bytes=0-1\r\n"));
        }
        let downgrade = Url::parse(&format!("https://127.0.0.1:{port}/downgrade")).unwrap();
        let response = tokio::time::timeout(BOUND, probe.get(&downgrade, "bytes=0-1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status, 302);
        server.abort();
    }
}
