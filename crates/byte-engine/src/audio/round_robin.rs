use crate::{audio::source::Source, core::Entity};

/// The `RoundRobin` struct represents a round-robin sample player.
pub struct RoundRobin {
	assets: Vec<String>,
	index: usize,
}

impl RoundRobin {
	pub fn new(assets: Vec<String>) -> Self {
		Self { assets, index: 0 }
	}

	pub fn get(&mut self) -> Option<&str> {
		let asset = self.assets.get(self.index);
		self.index = (self.index + 1) % self.assets.len();
		asset.map(|asset| asset.as_str())
	}

	pub fn get_assets(&self) -> &Vec<String> {
		&self.assets
	}
}

impl Entity for RoundRobin {}

impl Source for RoundRobin {}
