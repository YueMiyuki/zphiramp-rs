use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use zphira_catalog::{ChartIndex, ChartPool, Submissions};
use zphira_game::GameContext;
use zphira_ledger::{Admins, Points, RoundRecords};
use zphira_phira::PhiraClient;

use crate::jwt::JwtService;
use crate::ratelimit::LoginLimiter;
use crate::request::DEFAULT_MAX_REQUEST_SIZE;
use crate::transfer::DataTransfer;

pub trait PlayerDirectory: Send + Sync + 'static {
    fn player_name(&self, user_id: i32) -> Option<String>;
}

struct GamePlayers(GameContext);

impl PlayerDirectory for GamePlayers {
    fn player_name(&self, user_id: i32) -> Option<String> {
        if let Some(player) = self
            .0
            .online_players()
            .into_iter()
            .find(|p| p.id == user_id)
        {
            return Some(player.name);
        }
        let room = self.0.room(&self.0.player_room(user_id)?)?;
        room.players
            .into_iter()
            .chain(room.monitors)
            .find(|m| m.id == user_id)
            .map(|m| m.name)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoPlayers;

impl PlayerDirectory for NoPlayers {
    fn player_name(&self, _user_id: i32) -> Option<String> {
        None
    }
}

#[derive(Debug, Clone)]
pub struct HttpConfig {
    pub max_request_size: usize,
    pub max_import_size: usize,
    pub login_rate_limit: Option<u32>,
    pub login_window: Duration,
    pub name_lookup_timeout: Duration,
    pub shutdown_grace: Duration,
    pub idle_timeout: Duration,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            max_request_size: DEFAULT_MAX_REQUEST_SIZE,
            max_import_size: 64 * 1024 * 1024,
            login_rate_limit: Some(30),
            login_window: Duration::from_secs(60),
            name_lookup_timeout: Duration::from_secs(3),
            shutdown_grace: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(30),
        }
    }
}

pub struct AppState {
    pub(crate) data_dir: PathBuf,
    pub(crate) frontend_dir: PathBuf,
    pub(crate) phira: PhiraClient,
    pub(crate) pools: ChartPool,
    pub(crate) index: ChartIndex,
    pub(crate) submissions: Submissions,
    pub(crate) points: Points,
    pub(crate) records: RoundRecords,
    pub(crate) admins: Admins,
    pub(crate) jwt: JwtService,
    pub(crate) players: Arc<dyn PlayerDirectory>,
    pub(crate) game: Option<GameContext>,
    pub(crate) config: HttpConfig,
    pub(crate) login_limiter: LoginLimiter,
    pub(crate) transfer: DataTransfer,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("data_dir", &self.data_dir)
            .field("frontend_dir", &self.frontend_dir)
            .finish_non_exhaustive()
    }
}

pub struct Stores {
    pub phira: PhiraClient,
    pub pools: ChartPool,
    pub index: ChartIndex,
    pub submissions: Submissions,
    pub points: Points,
    pub records: RoundRecords,
    pub admins: Admins,
    pub jwt: JwtService,
}

pub struct AppStateBuilder {
    stores: Stores,
    data_dir: PathBuf,
    frontend_dir: PathBuf,
    players: Option<Arc<dyn PlayerDirectory>>,
    game: Option<GameContext>,
    config: HttpConfig,
}

impl AppState {
    pub fn builder(stores: Stores) -> AppStateBuilder {
        AppStateBuilder {
            stores,
            data_dir: PathBuf::from("data"),
            frontend_dir: PathBuf::from("frontend").join("dist"),
            players: None,
            game: None,
            config: HttpConfig::default(),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn jwt(&self) -> &JwtService {
        &self.jwt
    }
}

impl AppStateBuilder {
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = dir.into();
        self
    }

    pub fn frontend_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.frontend_dir = dir.into();
        self
    }

    pub fn game(mut self, game: GameContext) -> Self {
        self.game = Some(game);
        self
    }

    pub fn players(mut self, players: Arc<dyn PlayerDirectory>) -> Self {
        self.players = Some(players);
        self
    }

    pub fn config(mut self, config: HttpConfig) -> Self {
        self.config = config;
        self
    }

    pub fn build(self) -> Arc<AppState> {
        let frontend_dir = crate::static_files::absolute_normalized(&self.frontend_dir);
        let Stores {
            phira,
            pools,
            index,
            submissions,
            points,
            records,
            admins,
            jwt,
        } = self.stores;
        Arc::new(AppState {
            transfer: DataTransfer::new(self.data_dir.clone()),
            login_limiter: LoginLimiter::new(
                self.config.login_rate_limit,
                self.config.login_window,
            ),
            data_dir: self.data_dir,
            frontend_dir,
            phira,
            pools,
            index,
            submissions,
            points,
            records,
            admins,
            jwt,
            players: self.players.unwrap_or_else(|| match &self.game {
                Some(game) => Arc::new(GamePlayers(game.clone())),
                None => Arc::new(NoPlayers),
            }),
            game: self.game,
            config: self.config,
        })
    }
}
