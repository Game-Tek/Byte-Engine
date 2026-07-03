use std::time::Duration;

use crate::core::{listener::Listener, Entity, EntityHandle};

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
		Self { timers: Vec::new() }
	}

	pub fn update(&mut self, time: Duration) {
		for timer in &self.timers {}
	}
}
