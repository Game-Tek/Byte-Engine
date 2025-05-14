use crate::core::{Entity, EntityHandle, SpawnHandler};

use super::event::EventRegistry;

pub trait Domain {
	fn get_event_registry(&self) -> Option<&EventRegistry> {
		None
	}
}
