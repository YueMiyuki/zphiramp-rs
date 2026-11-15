//! Connections, sessions, rooms and the round state machine of the Phira multiplayer server.

mod api;
mod context;
mod lobby;
pub mod net;
mod player;
mod room;
mod services;
mod session;
pub mod text;
mod upstream;
mod util;

pub use api::{
    ChartApiView, CreateRoomError, EndRoundError, MemberView, PlayerView, PoolStatusView, PoolView,
    RemoveRoomError, RoomConfigView, RoomNotFound, RoomPools, RoomUpdate, RoomView,
    SwitchPoolError, UpdateRoomError, system_chat,
};
pub use context::{
    Background, GameContext, GameDeps, Listener, ListenerTask, TaskExit, TaskName, TaskOutcome,
};
pub use lobby::is_valid_room_id;
pub use room::store::{ROOMS_FILE, SavedRoom, SavedSetting};
pub use room::{
    DEFAULT_ALL_READY_DELAY_SECONDS, EMPTY_POOL_LIST, MAX_ALL_READY_DELAY_SECONDS, RoomChartPool,
    RoomSetting, Rotation, effective_force_finish_seconds,
};
pub use services::{GameOptions, WallClock};
pub use upstream::{BoxFuture, GameUpstream, UpstreamError};
pub use util::mask_token;
