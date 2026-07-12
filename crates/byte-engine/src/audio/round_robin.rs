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
		if self.assets.is_empty() {
			return None;
		}

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

#[cfg(test)]
mod tests {
	use super::RoundRobin;

	#[test]
	fn assets_repeat_in_insertion_order() {
		let mut source = RoundRobin::new(vec!["a".into(), "b".into(), "c".into()]);
		let sequence: Vec<_> = (0..8).map(|_| source.get().unwrap().to_string()).collect();

		assert_eq!(sequence, ["a", "b", "c", "a", "b", "c", "a", "b"]);
		assert_eq!(source.get_assets(), &["a", "b", "c"]);
	}

	#[test]
	fn empty_source_remains_empty_without_panicking() {
		let mut source = RoundRobin::new(Vec::new());
		assert_eq!(source.get(), None);
		assert_eq!(source.get(), None);
		assert!(source.get_assets().is_empty());
	}
}
