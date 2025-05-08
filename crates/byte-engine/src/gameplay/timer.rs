use std::time::Duration;

use crate::core::{entity::EntityBuilder, listener::EntitySubscriber, Entity, EntityHandle};

struct Timer {
	period: Duration,
}

impl Timer {
	pub fn new(period: Duration) -> Self {
		Self { period }
	}
}

impl Entity for Timer {}

struct TimerService {
	timers: Vec<EntityHandle<Timer>>,
}

impl TimerService {
	pub fn new() -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self {
			timers: Vec::new(),
		}).listen_to::<Timer>()
	}

	pub fn update(&mut self, time: Duration) {
		for timer in &self.timers {
			
		}
	}
}

impl EntitySubscriber<Timer> for TimerService {
	fn on_create(&mut self, handle: EntityHandle<Timer>, timer: &Timer) {
		self.timers.push(handle);
	}

	fn on_delete(&mut self, handle: EntityHandle<Timer>) {
		self.timers.retain(|h| *h != handle);
	}
}