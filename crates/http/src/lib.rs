//! HTTP admin API of the game server and static hosting of the web manager.

mod auth;
mod body;
mod java;
mod jwt;
mod ratelimit;
mod reply;
mod request;
mod router;
mod routes;
mod server;
mod state;
mod static_files;
mod transfer;
mod views;

pub use jwt::{EXPIRATION_MILLIS, JwtAlgorithm, JwtError, JwtService, SECRET_FILE};
pub use request::DEFAULT_MAX_REQUEST_SIZE;
pub use server::{BindError, HttpServer, ServerExit, StartError, router};
pub use state::{AppState, AppStateBuilder, HttpConfig, NoPlayers, PlayerDirectory, Stores};
pub use transfer::DataKind;
