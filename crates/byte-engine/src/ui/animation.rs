pub enum Curves {
	Linear,
}

pub struct Animation<V: Interpolate> {
	keyframes: Vec<(f32, V)>,
}

impl<V: Interpolate> Animation<V> {
	pub fn new() -> Self {
		Self { keyframes: Vec::new() }
	}

	pub fn add_keyframe(&mut self, time: f32, value: V) {
		self.keyframes.push((time, value));
	}
}

pub struct Track<V: Interpolate> {
	animation: Animation<V>,
	duration: f32,
	current_time: f32,
}

impl<V: Interpolate> Track<V> {
	pub fn new(animation: Animation<V>, duration: f32) -> Self {
		Self {
			animation,
			duration,
			current_time: 0.0,
		}
	}

	pub fn update(&mut self, dt: f32) -> V {
		self.current_time += dt;
		if self.current_time > self.duration {
			self.current_time = 0.0;
		}

		let mut keyframes = self.animation.keyframes.iter();
		let mut prev = keyframes.next().unwrap();
		for curr in keyframes {
			if self.current_time < curr.0 {
				return prev.1.interpolate(&curr.1, (self.current_time - prev.0) / (curr.0 - prev.0));
			}
			prev = curr;
		}
		prev.1.interpolate(&prev.1, 0.0)
	}
}

pub trait Interpolate {
	fn interpolate(&self, other: &Self, t: f32) -> Self;
}

impl Interpolate for f32 {
	fn interpolate(&self, other: &Self, t: f32) -> Self {
		self * (1.0 - t) + other * t
	}
}
