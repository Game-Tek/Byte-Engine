use crate::core::{Entity, EntityHandle, SpawnHandler};

use super::listener::BasicListener;

pub trait Domain {
	fn get_listener(&self) -> Option<&BasicListener>;
	fn get_listener_mut(&mut self) -> Option<&mut BasicListener>;
}
