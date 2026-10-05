/// The `LutDescription` struct captures the metadata needed to bake a parsed LUT into a resource.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LutDescription {
	pub kind: LutKind,
	pub size: u32,
	pub domain_min: [f32; 3],
	pub domain_max: [f32; 3],
}

pub fn process_lut<'a>(
	id: ResourceId<'a>,
	description: LutDescription,
	entries: Vec<[f32; 3]>,
) -> Result<(ProcessedAsset, Box<[u8]>), LoadErrors> {
	validate_lut(&description, &entries)?;

	let resource = Lut {
		kind: description.kind,
		size: description.size,
		domain_min: description.domain_min,
		domain_max: description.domain_max,
	};

	Ok((ProcessedAsset::new(id, resource), encode_entries(entries)))
}

/// Validates that the parsed LUT metadata and sample count are internally consistent.
fn validate_lut(description: &LutDescription, entries: &[[f32; 3]]) -> Result<(), LoadErrors> {
	if description.size == 0 {
		return Err(LoadErrors::FailedToProcess);
	}

	if description
		.domain_min
		.into_iter()
		.zip(description.domain_max)
		.any(|(minimum, maximum)| !minimum.is_finite() || !maximum.is_finite() || minimum >= maximum)
	{
		return Err(LoadErrors::FailedToProcess);
	}

	if entries
		.iter()
		.flat_map(|entry| entry.iter().copied())
		.any(|value| !value.is_finite())
	{
		return Err(LoadErrors::FailedToProcess);
	}

	let Some(expected_entry_count) = description.kind.expected_entry_count(description.size) else {
		return Err(LoadErrors::FailedToProcess);
	};

	if entries.len() != expected_entry_count {
		return Err(LoadErrors::FailedToProcess);
	}

	Ok(())
}

fn encode_entries(entries: Vec<[f32; 3]>) -> Box<[u8]> {
	let mut data = Vec::with_capacity(entries.len() * 3 * std::mem::size_of::<f32>());

	for [r, g, b] in entries {
		data.extend_from_slice(&r.to_le_bytes());

		data.extend_from_slice(&g.to_le_bytes());

		data.extend_from_slice(&b.to_le_bytes());
	}

	data.into_boxed_slice()
}

#[cfg(test)]
mod tests {
	use crate::{
		asset::ResourceId,
		processors::lut::{LutDescription, process_lut},
		resources::lut::LutKind,
	};

	#[test]
	fn process_lut_rejects_incorrect_entry_count() {
		let description = LutDescription {
			kind: LutKind::OneDimensional,
			size: 4,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};

		let result = process_lut(
			ResourceId::new("grading/invalid.lut"),
			description,
			vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]],
		);

		assert!(matches!(result, Err(crate::asset::handler::LoadErrors::FailedToProcess)));
	}
}

use crate::{
	ProcessedAsset,
	asset::{ResourceId, handler::LoadErrors},
	resources::lut::{Lut, LutKind},
};
