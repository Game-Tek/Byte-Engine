use crate::{
	core::{listener::Listener, Entity, EntityHandle},
	time::MediaTime,
};

struct Timer {
	period: MediaTime,
}

impl Timer {
	pub fn new(period: MediaTime) -> Self {
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

	pub fn update(&mut self, time: MediaTime) {
		for timer in &self.timers {}
	}
}
