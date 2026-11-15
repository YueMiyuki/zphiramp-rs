use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use zphira_common::error::GameError;

use super::Room;
use crate::util::lock;

#[derive(Debug, Default)]
pub(crate) struct RoomRegistry {
    rooms: Mutex<BTreeMap<String, Arc<Room>>>,
}

impl RoomRegistry {
    pub(crate) fn create(
        &self,
        room_id: &str,
        max_rooms: usize,
        build: impl FnOnce() -> Arc<Room>,
    ) -> Result<Arc<Room>, GameError> {
        let mut rooms = lock(&self.rooms);
        if let Some(existing) = rooms.get(room_id)
            && existing.is_destroyed()
        {
            rooms.remove(room_id);
        }
        if rooms.len() >= max_rooms {
            return Err(GameError::room_limit_reached(
                i32::try_from(max_rooms).unwrap_or(i32::MAX),
            ));
        }
        if rooms.contains_key(room_id) {
            return Err(GameError::room_already_exists());
        }
        let room = build();
        rooms.insert(room_id.to_owned(), room.clone());
        Ok(room)
    }

    pub(crate) fn find(&self, room_id: &str) -> Option<Arc<Room>> {
        lock(&self.rooms).get(room_id).cloned()
    }

    pub(crate) fn all(&self) -> Vec<Arc<Room>> {
        lock(&self.rooms).values().cloned().collect()
    }

    pub(crate) fn remove_if_same(&self, room_id: &str, room: &Room) {
        let mut rooms = lock(&self.rooms);
        if rooms
            .get(room_id)
            .is_some_and(|r| std::ptr::eq(Arc::as_ptr(r), room))
        {
            rooms.remove(room_id);
        }
    }
}
