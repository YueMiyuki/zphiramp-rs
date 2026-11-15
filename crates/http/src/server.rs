use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use http::Request;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use hyper_util::server::graceful::GracefulShutdown;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tower::ServiceExt;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{NotForContentType, Predicate, SizeAbove};

use crate::router::dispatch;
use crate::state::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    let predicate = SizeAbove::new(1500).and(NotForContentType::IMAGES);
    Router::new()
        .fallback(move |request: Request<Body>| {
            let state = state.clone();
            async move { dispatch(state, request).await }
        })
        .layer(CompressionLayer::new().compress_when(predicate))
}

#[derive(Debug)]
pub struct BindError {
    port: u16,
    source: io::Error,
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.source.kind() == io::ErrorKind::AddrInUse {
            write!(
                f,
                "Port already in use. Make sure no other process is using port {} and try again.",
                self.port
            )
        } else {
            fmt::Display::fmt(&self.source, f)
        }
    }
}

impl std::error::Error for BindError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

#[derive(Debug)]
pub struct StartError {
    host: String,
    port: u16,
    cause: Option<BindError>,
}

impl fmt::Display for StartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HTTP API server failed to bind {}:{}; the game port is up but the web manager is unreachable. Free the port or pass a different --http-port.",
            self.host, self.port
        )
    }
}

impl std::error::Error for StartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause
            .as_ref()
            .map(|c| c as &(dyn std::error::Error + 'static))
    }
}

pub struct HttpServer {
    host: String,
    port: u16,
    started: Option<oneshot::Receiver<Result<SocketAddr, BindError>>>,
    outcome: Option<Result<SocketAddr, ()>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<TaskEnd>>,
    exit: Option<ServerExit>,
    grace: Duration,
}

enum TaskEnd {
    Served,
    BindFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerExit {
    Stopped,
    BindFailed,
    Panicked(String),
    Aborted,
}

impl fmt::Display for ServerExit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => f.write_str("stopped"),
            Self::BindFailed => f.write_str("failed to bind"),
            Self::Panicked(message) => write!(f, "panicked: {message}"),
            Self::Aborted => f.write_str("aborted"),
        }
    }
}

fn exit_of(result: Result<TaskEnd, tokio::task::JoinError>) -> ServerExit {
    match result {
        Ok(TaskEnd::Served) => ServerExit::Stopped,
        Ok(TaskEnd::BindFailed) => ServerExit::BindFailed,
        Err(e) if e.is_panic() => {
            let payload = e.into_panic();
            let message = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_owned());
            ServerExit::Panicked(message)
        }
        Err(_) => ServerExit::Aborted,
    }
}

impl fmt::Debug for HttpServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpServer")
            .field("host", &self.host)
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl HttpServer {
    pub fn start(state: Arc<AppState>, host: impl Into<String>, port: u16) -> HttpServer {
        let host = host.into();
        let (started_tx, started_rx) = oneshot::channel();
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let grace = state.config.shutdown_grace;
        let idle_timeout = state.config.idle_timeout;
        let bind_host = host.clone();
        let task = tokio::spawn(async move {
            let listener = match TcpListener::bind((bind_host.as_str(), port)).await {
                Ok(listener) => listener,
                Err(source) => {
                    let error = BindError { port, source };
                    tracing::error!(
                        error = &error as &dyn std::error::Error,
                        "HTTP API server failed to bind {}:{}",
                        bind_host,
                        port
                    );
                    let _ = started_tx.send(Err(error));
                    return TaskEnd::BindFailed;
                }
            };
            let local = listener
                .local_addr()
                .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], port)));
            tracing::info!("HTTP API server listening on {}:{}", bind_host, port);
            let _ = started_tx.send(Ok(local));
            serve(listener, router(state), shutdown_rx, grace, idle_timeout).await;
            TaskEnd::Served
        });
        HttpServer {
            host,
            port,
            started: Some(started_rx),
            outcome: None,
            shutdown: Some(shutdown_tx),
            task: Some(task),
            exit: None,
            grace,
        }
    }

    pub async fn await_started(&mut self) -> Result<SocketAddr, StartError> {
        if self.outcome.is_none() {
            let outcome = match self.started.take() {
                Some(rx) => rx.await.unwrap_or_else(|_| {
                    Err(BindError {
                        port: self.port,
                        source: io::Error::other("the HTTP server task ended before binding"),
                    })
                }),
                None => Err(BindError {
                    port: self.port,
                    source: io::Error::other("already reported"),
                }),
            };
            match outcome {
                Ok(addr) => self.outcome = Some(Ok(addr)),
                Err(cause) => {
                    self.outcome = Some(Err(()));
                    return Err(StartError {
                        host: self.host.clone(),
                        port: self.port,
                        cause: Some(cause),
                    });
                }
            }
        }
        match self.outcome {
            Some(Ok(addr)) => Ok(addr),
            _ => Err(StartError {
                host: self.host.clone(),
                port: self.port,
                cause: None,
            }),
        }
    }

    /// Waits until the server has ended without `stop` and says why; pending while it runs.
    pub async fn closed(&mut self) -> ServerExit {
        if let Some(exit) = &self.exit {
            return exit.clone();
        }
        let Some(task) = self.task.as_mut() else {
            return ServerExit::Stopped;
        };
        let exit = exit_of(task.await);
        self.task = None;
        self.exit = Some(exit.clone());
        exit
    }

    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(mut task) = self.task.take()
            && tokio::time::timeout(self.grace + Duration::from_secs(1), &mut task)
                .await
                .is_err()
        {
            task.abort();
        }
    }
}

async fn serve(
    listener: TcpListener,
    router: Router,
    mut shutdown: oneshot::Receiver<()>,
    grace: Duration,
    idle_timeout: Duration,
) {
    let graceful = GracefulShutdown::new();
    loop {
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(e) => {
                    // Out of file descriptors and the like: back off instead of spinning.
                    tracing::debug!(error = %e, "accept failed");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                }
            },
            _ = &mut shutdown => break,
        };
        let router = router.clone();
        let service = service_fn(move |mut request: Request<Incoming>| {
            request.extensions_mut().insert(ConnectInfo(peer));
            router.clone().oneshot(request.map(Body::new))
        });
        let connection = http1::Builder::new()
            .timer(TokioTimer::new())
            .header_read_timeout(idle_timeout)
            .serve_connection(TokioIo::new(stream), service);
        let connection = graceful.watch(connection);
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::debug!(error = %e, "HTTP connection ended with an error");
            }
        });
    }
    drop(listener);
    if tokio::time::timeout(grace, graceful.shutdown())
        .await
        .is_err()
    {
        tracing::debug!("HTTP requests still running after the grace period were dropped");
    }
}

impl Drop for HttpServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exits_of_a_task() {
        let served = tokio::spawn(async { TaskEnd::Served });
        assert_eq!(exit_of(served.await), ServerExit::Stopped);
        let panicked = tokio::spawn(async {
            if true {
                panic!("accept loop broke");
            }
            TaskEnd::Served
        });
        assert_eq!(
            exit_of(panicked.await),
            ServerExit::Panicked("accept loop broke".into())
        );
        let formatted = tokio::spawn(async {
            if true {
                panic!("code {}", 7);
            }
            TaskEnd::Served
        });
        assert_eq!(
            exit_of(formatted.await),
            ServerExit::Panicked("code 7".into())
        );
        let pending = tokio::spawn(async {
            std::future::pending::<()>().await;
            TaskEnd::Served
        });
        pending.abort();
        assert_eq!(exit_of(pending.await), ServerExit::Aborted);
    }
}
