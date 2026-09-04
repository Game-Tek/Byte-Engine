//! Stable shader-table slot assignment shared by visibility resource families.

use utils::hash::HashMap;

/// Returns or assigns a stable slot for `id`, failing once `limit` slots exist.
pub(super) fn assign_slot(slots: &mut HashMap<String, u32>, id: &str, limit: usize, kind: &str) -> Option<u32> {
	if let Some(index) = slots.get(id) {
		return Some(*index);
	}
	if slots.len() >= limit {
		log::error!(
			"Visibility {kind} limit exceeded. The most likely cause is that the scene created more {kind} variants than the visibility pipeline supports."
		);
		return None;
	}
	let index = slots.len() as u32;
	slots.insert(id.to_string(), index);
	Some(index)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn slots_are_stable_and_bounded() {
		let mut slots = HashMap::default();
		assert_eq!(assign_slot(&mut slots, "a", 2, "test"), Some(0));
		assert_eq!(assign_slot(&mut slots, "b", 2, "test"), Some(1));
		assert_eq!(assign_slot(&mut slots, "a", 2, "test"), Some(0));
		assert_eq!(assign_slot(&mut slots, "c", 2, "test"), None);
	}
}
