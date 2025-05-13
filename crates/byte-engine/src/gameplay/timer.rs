use std::time::Duration;

use crate::core::{entity::EntityBuilder, listener::{CreateEvent, Listener}, Entity, EntityHandle};

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
	pub fn new() -> Self {
		Self {
			timers: Vec::new(),
		}
	}

	pub fn update(&mut self, time: Duration) {
		for timer in &self.timers {
			
		}
	}
}

impl Entity for TimerService {
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self).listen_to::<CreateEvent<Timer>>()
	}
}

impl Listener<CreateEvent<Timer>> for TimerService {
	fn handle(&mut self, event: &CreateEvent<Timer>) {
		let handle = event.handle();
		self.timers.push(handle.clone());
	}
}