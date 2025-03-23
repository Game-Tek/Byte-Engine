use crate::core::{Entity, EntityHandle};

struct Killer {
	entities: Vec<Data>,
}

struct Data {
	entity: EntityHandle<dyn Entity>,
}

impl Killer {
	pub fn new() -> Self {
		Self {
			entities: Vec::with_capacity(256),
		}
	}

	pub fn add_entity(&mut self, entity: EntityHandle<dyn Entity>) {
		self.entities.push(Data {
			entity,
		});
	}
}