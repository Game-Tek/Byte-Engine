use crate::{
	asset,
	processors::lut_processor::{process_lut, LutDescription},
	r#async::{spawn_cpu_task, BoxedFuture},
	resource,
	resources::lut::LutKind,
	ProcessedAsset,
};

use super::{
	asset_handler::{AssetHandler, LoadErrors},
	asset_manager::AssetManager,
	ResourceId,
};

#[derive(Debug)]
struct ParsedLut {
	description: LutDescription,
	entries: Vec<[f32; 3]>,
}

/// The `LUTAssetHandler` struct bakes text lookup-table assets into LUT resources.
pub struct LUTAssetHandler {}

impl LUTAssetHandler {
	pub fn new() -> LUTAssetHandler {
		LUTAssetHandler {}
	}

	/// Parses the textual LUT description into normalized metadata and sample entries.
	fn parse_lut(data: &[u8]) -> Result<ParsedLut, String> {
		let source = std::str::from_utf8(data).map_err(|error| {
			format!("Invalid LUT text. The most likely cause is that the .lut file is binary or not UTF-8 encoded: {error}")
		})?;

		let mut kind = None;
		let mut size = None;
		let mut domain_min = [0.0, 0.0, 0.0];
		let mut domain_max = [1.0, 1.0, 1.0];
		let mut entries = Vec::new();

		for (line_index, raw_line) in source.lines().enumerate() {
			let line_number = line_index + 1;
			let line = raw_line.split('#').next().unwrap_or("").trim();
			if line.is_empty() {
				continue;
			}

			let tokens = line.split_whitespace().collect::<Vec<_>>();
			match tokens[0] {
				"TITLE" => {}
				"LUT_1D_SIZE" => {
					let parsed_size = parse_size_directive(&tokens, line_number, "LUT_1D_SIZE")?;
					assign_kind_and_size(&mut kind, &mut size, LutKind::OneDimensional, parsed_size, line_number)?;
				}
				"LUT_3D_SIZE" => {
					let parsed_size = parse_size_directive(&tokens, line_number, "LUT_3D_SIZE")?;
					assign_kind_and_size(&mut kind, &mut size, LutKind::ThreeDimensional, parsed_size, line_number)?;
				}
				"DOMAIN_MIN" => {
					domain_min = parse_triplet_tokens(&tokens, line_number, "DOMAIN_MIN")?;
				}
				"DOMAIN_MAX" => {
					domain_max = parse_triplet_tokens(&tokens, line_number, "DOMAIN_MAX")?;
				}
				_ => {
					entries.push(parse_entry_tokens(&tokens, line_number)?);
				}
			}
		}

		let (kind, size) = match (kind, size) {
			(Some(kind), Some(size)) => (kind, size),
			_ => {
				return Err(
					"Missing LUT size directive. The most likely cause is that the file does not declare LUT_1D_SIZE or LUT_3D_SIZE."
						.to_string(),
				);
			}
		};

		Ok(ParsedLut {
			description: LutDescription {
				kind,
				size,
				domain_min,
				domain_max,
			},
			entries,
		})
	}
}

impl AssetHandler for LUTAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		r#type == "lut"
	}

	fn bake<'a>(
		&'a self,
		_: &'a AssetManager,
		storage_backend: &'a dyn resource::StorageBackend,
		asset_storage_backend: &'a dyn asset::StorageBackend,
		url: ResourceId<'a>,
	) -> BoxedFuture<'a, Result<(ProcessedAsset, Box<[u8]>), LoadErrors>> {
		Box::pin(async move {
			if let Some(dt) = storage_backend.get_type(url) {
				if !self.can_handle(dt) {
					return Err(LoadErrors::UnsupportedType);
				}
			}

			let (data, _, dt) = asset_storage_backend
				.resolve(url)
				.await
				.or(Err(LoadErrors::AssetCouldNotBeLoaded))?;

			if !self.can_handle(&dt) {
				return Err(LoadErrors::UnsupportedType);
			}

			let parsed = spawn_cpu_task(move || Self::parse_lut(&data))
				.await
				.map_err(|_| LoadErrors::FailedToProcess)?
				.map_err(|_| LoadErrors::FailedToProcess)?;

			process_lut(url, parsed.description, parsed.entries)
		})
	}
}

fn parse_size_directive(tokens: &[&str], line_number: usize, keyword: &str) -> Result<u32, String> {
	if tokens.len() != 2 {
		return Err(format!(
			"Invalid {keyword} directive on line {line_number}. The most likely cause is that the size value is missing."
		));
	}

	let size = tokens[1].parse::<u32>().map_err(|error| {
		format!(
			"Invalid {keyword} directive on line {line_number}. The most likely cause is that the size is not a positive integer: {error}"
		)
	})?;

	if size == 0 {
		return Err(format!(
			"Invalid {keyword} directive on line {line_number}. The most likely cause is that the LUT size is zero."
		));
	}

	Ok(size)
}

fn assign_kind_and_size(
	kind: &mut Option<LutKind>,
	size: &mut Option<u32>,
	next_kind: LutKind,
	next_size: u32,
	line_number: usize,
) -> Result<(), String> {
	if kind.is_some() || size.is_some() {
		return Err(format!(
			"Duplicate LUT size directive on line {line_number}. The most likely cause is that the file declares more than one LUT size."
		));
	}

	*kind = Some(next_kind);
	*size = Some(next_size);

	Ok(())
}

fn parse_triplet_tokens(tokens: &[&str], line_number: usize, context: &str) -> Result<[f32; 3], String> {
	if tokens.len() != 4 {
		return Err(format!(
			"Invalid LUT {context} on line {line_number}. The most likely cause is that the line does not contain exactly three float values."
		));
	}

	let mut values = [0.0; 3];

	for (index, value) in values.iter_mut().enumerate() {
		*value = tokens[index + 1].parse::<f32>().map_err(|error| {
			format!(
				"Invalid LUT {context} on line {line_number}. The most likely cause is that one of the values is not a valid float: {error}"
			)
		})?;
	}

	if values.into_iter().any(|value| !value.is_finite()) {
		return Err(format!(
			"Invalid LUT {context} on line {line_number}. The most likely cause is that one of the values is NaN or infinite."
		));
	}

	Ok(values)
}

fn parse_entry_tokens(tokens: &[&str], line_number: usize) -> Result<[f32; 3], String> {
	if tokens.len() != 3 {
		return Err(format!(
			"Invalid LUT entry on line {line_number}. The most likely cause is that the line does not contain exactly three float values."
		));
	}

	let mut values = [0.0; 3];

	for (index, value) in values.iter_mut().enumerate() {
		*value = tokens[index].parse::<f32>().map_err(|error| {
			format!(
				"Invalid LUT entry on line {line_number}. The most likely cause is that one of the values is not a valid float: {error}"
			)
		})?;
	}

	if values.into_iter().any(|value| !value.is_finite()) {
		return Err(format!(
			"Invalid LUT entry on line {line_number}. The most likely cause is that one of the values is NaN or infinite."
		));
	}

	Ok(values)
}

#[cfg(test)]
mod tests {
	use crate::{
		asset::{
			self, asset_handler::AssetHandler, asset_manager::AssetManager, lut_asset_handler::LUTAssetHandler, ResourceId,
		},
		r#async, resource,
		resources::lut::{Lut, LutKind},
	};

	#[test]
	fn parse_lut_supports_domain_directives_and_comments() {
		let lut = br#"
			# Neutral 1D LUT
			TITLE "Neutral"
			LUT_1D_SIZE 2
			DOMAIN_MIN -1.0 -1.0 -1.0
			DOMAIN_MAX 2.0 2.0 2.0
			0.0 0.0 0.0
			1.0 1.0 1.0
		"#;

		let parsed = LUTAssetHandler::parse_lut(lut).expect("LUT should parse");

		assert_eq!(parsed.description.kind, LutKind::OneDimensional);
		assert_eq!(parsed.description.size, 2);
		assert_eq!(parsed.description.domain_min, [-1.0, -1.0, -1.0]);
		assert_eq!(parsed.description.domain_max, [2.0, 2.0, 2.0]);
		assert_eq!(parsed.entries, vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]]);
	}

	#[test]
	fn parse_lut_rejects_duplicate_size_directives() {
		let lut = br#"
			LUT_1D_SIZE 2
			LUT_3D_SIZE 2
			0.0 0.0 0.0
			1.0 1.0 1.0
		"#;

		let error = LUTAssetHandler::parse_lut(lut).expect_err("LUT should fail to parse");

		assert!(error.starts_with("Duplicate LUT size directive"));
	}

	#[r#async::test]
	async fn bake_lut_asset_generates_lut_resource() {
		let asset_handler = LUTAssetHandler::new();

		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		asset_storage_backend.add_file(
			"grading/neutral.lut",
			br#"
				LUT_3D_SIZE 2
				0.0 0.0 0.0
				1.0 0.0 0.0
				0.0 1.0 0.0
				1.0 1.0 0.0
				0.0 0.0 1.0
				1.0 0.0 1.0
				0.0 1.0 1.0
				1.0 1.0 1.0
			"#,
		);

		let resource_storage_backend = resource::storage_backend::tests::TestStorageBackend::new();
		let asset_manager = AssetManager::new(asset_storage_backend.clone());

		let (resource, data) = asset_handler
			.bake(
				&asset_manager,
				&resource_storage_backend,
				&asset_storage_backend,
				ResourceId::new("grading/neutral.lut"),
			)
			.await
			.expect("LUT asset handler should bake the asset");

		crate::resource::WriteStorageBackend::store(&resource_storage_backend, &resource, &data)
			.expect("LUT resource should store");

		let generated = resource_storage_backend
			.get_resource(ResourceId::new("grading/neutral.lut"))
			.expect("LUT resource should exist");
		let lut: Lut = crate::from_slice(&generated.resource).expect("Stored resource should deserialize as a LUT");

		assert_eq!(generated.class, "Lut");
		assert_eq!(lut.kind, LutKind::ThreeDimensional);
		assert_eq!(lut.size, 2);
		assert_eq!(data.len(), 8 * 3 * std::mem::size_of::<f32>());
	}
}
