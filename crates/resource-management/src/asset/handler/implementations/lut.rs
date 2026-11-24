#[derive(Debug)]
struct ParsedLut {
	description: LutDescription,
	entries: Vec<[f32; 3]>,
}

/// The `LUTAssetHandler` struct bakes `.lut` and `.cube` lookup-table assets into LUT resources.
#[derive(Default)]
pub struct LUTAssetHandler {}

impl LUTAssetHandler {
	pub fn new() -> LUTAssetHandler {
		Self::default()
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

			// Parse directly from the line so large LUTs do not allocate a token vector for every sample.
			let mut tokens = line.split_whitespace();

			let first = tokens.next().expect("non-empty LUT lines contain at least one token");

			match first {
				"TITLE" => {}
				"LUT_1D_SIZE" => {
					let parsed_size = parse_size_directive(tokens, line_number, "LUT_1D_SIZE")?;

					assign_kind_and_size(&mut kind, &mut size, LutKind::OneDimensional, parsed_size, line_number)?;
				}
				"LUT_3D_SIZE" => {
					let parsed_size = parse_size_directive(tokens, line_number, "LUT_3D_SIZE")?;

					assign_kind_and_size(&mut kind, &mut size, LutKind::ThreeDimensional, parsed_size, line_number)?;
				}
				"DOMAIN_MIN" => {
					domain_min = parse_triplet_tokens(tokens, line_number, "DOMAIN_MIN")?;
				}
				"DOMAIN_MAX" => {
					domain_max = parse_triplet_tokens(tokens, line_number, "DOMAIN_MAX")?;
				}
				_ => {
					entries.push(parse_entry_tokens(first, tokens, line_number)?);
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
		matches!(r#type, "lut" | "cube")
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<PrimaryStored, LoadErrors> {
		let (data, dt) = context.resolve(url).await?;

		if !self.can_handle(&dt) {
			return Err(LoadErrors::UnsupportedType);
		}

		// The source bytes borrow the bake allocator, so parsing stays in this task.
		let parsed = Self::parse_lut(&data).map_err(|_| LoadErrors::FailedToProcess)?;

		let fail = |message: String| {
			context.error(format_args!("{message} See {}.", crate::online_docs_url(LUT_DOCS_PATH)));
			LoadErrors::FailedToProcess
		};
		let sidecar = context.load_sidecar(url).await?;
		let settings = lut_settings(sidecar.as_ref()).map_err(|error| {
			fail(format!(
				"The settings of LUT '{}' are invalid: {error}. The most likely cause is a mistyped `{LUT_SETTINGS_KEY}` object in its BEAD file. Write it as `{{ {LUT_SETTINGS_KEY}: {{ through: \"<pixel shader>\", size: 65 }} }}`.",
				url.as_ref()
			))
		})?;

		let (description, entries) = match settings {
			Some(settings) => bake_through(&context, url, &parsed, &settings).await.map_err(fail)?,
			None => (parsed.description, parsed.entries),
		};
		let (resource, data) = process_lut(url, description, entries)?;

		context.store_primary_owned(resource, data).await
	}
}

/// The reference section that defines LUT files and their sidecar.
const LUT_DOCS_PATH: &str = "reference/assets#color-lookup-tables";

/// The sidecar key that holds a LUT's bake settings.
const LUT_SETTINGS_KEY: &str = "lut";

/// The bake-only pixel shader that turns a lattice position into the scene color the baked table is asked about.
const LATTICE_SHADER: &str = "byte_engine/rendering/color-grading/lattice.besl";

/// ACEScct of 0 and of 65,504, the largest half float: the ends of a baked table's lattice on every axis.
const LATTICE_MIN: f32 = 0.072_905_534;
const LATTICE_MAX: f32 = 1.467_996_3;

/// The tile the bake runs the chain on. The VM only checks that a pixel writes inside its own tile.
const BAKE_TILE: u32 = 16;

/// The instructions one lattice color may run before the bake gives up, far above what an output transform needs.
const BAKE_INSTRUCTION_LIMIT: usize = 1 << 20;

/// The `LutSettings` struct is what a LUT's sidecar holds under `lut`: the request to bake the table through a shader.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct LutSettings {
	/// The pixel shader that takes the `Lut` layer and whose result the baked table holds.
	through: String,
	/// The baked table's entries per side.
	#[serde(default = "default_baked_size")]
	size: u32,
}

fn default_baked_size() -> u32 {
	65
}

/// Reads the bake settings a sidecar gives a LUT, or `None` when it gives none.
fn lut_settings(spec: Option<&BEADType>) -> Result<Option<LutSettings>, String> {
	let Some(value) = spec.and_then(|spec| spec.get(LUT_SETTINGS_KEY)) else {
		return Ok(None);
	};
	let settings = <LutSettings as serde::Deserialize>::deserialize(value).map_err(|error| error.to_string())?;
	if !(2..=256).contains(&settings.size) {
		return Err(format!("`size` is {}, outside 2 to 256", settings.size));
	}
	Ok(Some(settings))
}

/// Returns the ACEScct lattice coordinate of entry `index` of a table with `size` entries per side.
fn lattice_coordinate(index: u32, size: u32) -> f32 {
	LATTICE_MIN + (LATTICE_MAX - LATTICE_MIN) * index as f32 / (size - 1) as f32
}

/// Bakes the table of `settings.size` entries per side that holds, for every lattice color, the result of the pixel
/// shader `settings.through` run with `look` as its LUT.
///
/// The lattice is ACEScct-encoded ACEScg from 0 to 65,504. The chain is the engine's lattice step, which decodes a
/// lattice position to scene-linear sRGB, then the shader. It runs in the BESL VM, one slice of the table at a time,
/// so the table holds exactly what the shader computes.
async fn bake_through(
	context: &BakeContext<'_>,
	url: ResourceId<'_>,
	look: &ParsedLut,
	settings: &LutSettings,
) -> Result<(LutDescription, Vec<[f32; 3]>), String> {
	let id = url.as_ref();
	if look.description.kind != LutKind::ThreeDimensional {
		return Err(format!(
			"LUT '{id}' can't be baked through a shader. The most likely cause is a 1D table, which the `Lut` layer doesn't read. Bake a `LUT_3D_SIZE` table, or remove `{LUT_SETTINGS_KEY}` from its BEAD file."
		));
	}

	let mut sources = Vec::with_capacity(2);
	for shader in [LATTICE_SHADER, settings.through.as_str()] {
		let (bytes, _) = context.resolve(ResourceId::new(shader)).await.map_err(|_| {
			format!(
				"Shader '{shader}', which LUT '{id}' is baked through, could not be read. The most likely cause is a `through` that names a file that doesn't exist. Point `through` at a pixel shader's `.besl` file."
			)
		})?;
		sources.push(String::from_utf8(bytes.to_vec()).map_err(|_| {
			format!(
				"Shader '{shader}' is not UTF-8 text. The most likely cause is a binary file saved with a `.besl` extension. Save the shader as UTF-8 BESL source."
			)
		})?);
	}
	let steps = [
		(ResourceId::new(LATTICE_SHADER), sources[0].as_str()),
		(ResourceId::new(&settings.through), sources[1].as_str()),
	];
	let could_not_link = |error: String| {
		format!(
			"LUT '{id}' could not be baked through '{}': {error} The most likely cause is a shader that isn't a pixel shader taking the `Lut` layer. Name a shader such as `byte_engine/rendering/color-grading/aces2.besl`.",
			settings.through
		)
	};
	let (wrapper, modules) = load_pixel_chain(context, &steps).await.map_err(could_not_link)?;
	let program = chain_program(&wrapper, &modules, context.program_transform()).map_err(could_not_link)?;
	// The table holds what BESL's own code computes, so a change of that code makes it stale.
	context.setting(SHADER_CODE_SETTING);
	let program = besl::lex(&context.asset_manager.besl_builtins, program).map_err(|error| could_not_link(format!("{error:?}.")))?;
	let program = ExecutableProgram::compile(&program).map_err(|error| could_not_link(format!("{error:?}.")))?;

	let missing = |name: &str| could_not_link(format!("The chain declares no `{name}`."));
	let source_slot = program.slot("source").ok_or_else(|| missing("source"))?;
	let texture_slot = program.slot("lut_texture").ok_or_else(|| missing("lut_texture"))?;
	let parameters_slot = program.slot("lut_parameters").ok_or_else(|| missing("lut_parameters"))?;
	let vm_failed = |error: VmError| could_not_link(format!("{error:?}."));

	// The look, as the shader samples it: red changes fastest, as in the file.
	let look_size = look.description.size;
	let mut look_texture = Texture::new_3d(look_size, look_size, look_size).map_err(vm_failed)?;
	for (index, entry) in look.entries.iter().enumerate() {
		let index = index as u32;
		let coordinate = [
			index % look_size,
			index / look_size % look_size,
			index / (look_size * look_size),
		];
		look_texture
			.write_3d(coordinate, [entry[0], entry[1], entry[2], 1.0])
			.map_err(vm_failed)?;
	}
	let look_resource = Lut {
		kind: look.description.kind,
		size: look_size,
		domain_min: look.description.domain_min,
		domain_max: look.description.domain_max,
	};
	let look_parameters = look_resource.parameters(1.0);
	let mut parameters = Buffer::new(
		program
			.buffer_layout(parameters_slot)
			.ok_or_else(|| missing("lut_parameters"))?
			.clone(),
	);
	for (name, value) in [
		("domain_min", look_parameters.domain_min),
		("domain_scale", look_parameters.domain_scale),
		("sampling", look_parameters.sampling),
	] {
		parameters.write(name, Value::Vec4F(value)).map_err(vm_failed)?;
	}
	// Both steps of the chain run for every lattice color.
	let mut mask = Buffer::new(program.push_constant_layout().ok_or_else(|| missing("steps"))?.clone());
	mask.write("steps", Value::U32(0b11)).map_err(vm_failed)?;

	let size = settings.size;
	let mut entries = Vec::with_capacity((size as usize).pow(3));
	let mut source = Texture::new(size, size).map_err(vm_failed)?;
	let mut result = Texture::new(size, size).map_err(vm_failed)?;
	// One slice of the table at a time: red along a row, green down the rows, blue from slice to slice.
	for blue in 0..size {
		for green in 0..size {
			for red in 0..size {
				let position = [red, green, blue].map(|index| lattice_coordinate(index, size));
				source
					.write([red, green], [position[0], position[1], position[2], 1.0])
					.map_err(vm_failed)?;
			}
		}
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(source_slot, &mut source);
		descriptors.bind_texture(texture_slot, &mut look_texture);
		descriptors.bind_buffer(parameters_slot, &mut parameters);
		descriptors.bind_push_constant(&mut mask);
		descriptors.bind_image(output_slot(0), &mut result);
		for green in 0..size {
			for red in 0..size {
				let config = ExecutionConfig::new(BAKE_INSTRUCTION_LIMIT)
					.with_thread_idx((green % BAKE_TILE) * BAKE_TILE + red % BAKE_TILE)
					.with_thread_id([red, green])
					.with_tile(TileInvocation {
						size: BAKE_TILE,
						position: [red / BAKE_TILE, green / BAKE_TILE],
						extent: [size, size],
					});
				program.run_main_with_config(&mut descriptors, &config).map_err(vm_failed)?;
			}
		}
		drop(descriptors);
		for green in 0..size {
			for red in 0..size {
				let Value::Vec4F(color) = result.fetch([red, green]).map_err(vm_failed)? else {
					return Err(could_not_link("The chain wrote something other than a color.".to_string()));
				};
				entries.push([color[0], color[1], color[2]]);
			}
		}
	}

	let description = LutDescription {
		kind: LutKind::ThreeDimensional,
		size,
		domain_min: [LATTICE_MIN; 3],
		domain_max: [LATTICE_MAX; 3],
	};
	Ok((description, entries))
}

fn parse_size_directive(mut tokens: std::str::SplitWhitespace<'_>, line_number: usize, keyword: &str) -> Result<u32, String> {
	let value = tokens.next();

	if value.is_none() || tokens.next().is_some() {
		return Err(format!(
			"Invalid {keyword} directive on line {line_number}. The most likely cause is that the size value is missing."
		));
	}

	let value = value.expect("the LUT size token was checked above");

	let size = value.parse::<u32>().map_err(|error| {
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

fn parse_triplet_tokens(
	mut tokens: std::str::SplitWhitespace<'_>,
	line_number: usize,
	context: &str,
) -> Result<[f32; 3], String> {
	let mut values = [0.0; 3];

	for value in &mut values {
		let token = tokens.next().ok_or_else(|| {
			format!(
				"Invalid LUT {context} on line {line_number}. The most likely cause is that the line does not contain exactly three float values."
			)
		})?;

		*value = token.parse::<f32>().map_err(|error| {
			format!(
				"Invalid LUT {context} on line {line_number}. The most likely cause is that one of the values is not a valid float: {error}"
			)
		})?;
	}

	if tokens.next().is_some() {
		return Err(format!(
			"Invalid LUT {context} on line {line_number}. The most likely cause is that the line does not contain exactly three float values."
		));
	}

	if values.into_iter().any(|value| !value.is_finite()) {
		return Err(format!(
			"Invalid LUT {context} on line {line_number}. The most likely cause is that one of the values is NaN or infinite."
		));
	}

	Ok(values)
}

fn parse_entry_tokens(first: &str, mut tokens: std::str::SplitWhitespace<'_>, line_number: usize) -> Result<[f32; 3], String> {
	let second = tokens.next();

	let third = tokens.next();

	if second.is_none() || third.is_none() || tokens.next().is_some() {
		return Err(format!(
			"Invalid LUT entry on line {line_number}. The most likely cause is that the line does not contain exactly three float values."
		));
	}

	let mut values = [0.0; 3];

	for (value, token) in values.iter_mut().zip([first, second.unwrap(), third.unwrap()]) {
		*value = token.parse::<f32>().map_err(|error| {
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

	use crate::{asset::handler::implementations::lut::LUTAssetHandler, resources::lut::LutKind};

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
}

use besl::vm::{
	Buffer, DescriptorBindings, ExecutableProgram, ExecutionConfig, Texture, TileInvocation, Value, VmError, output_slot,
};

use super::{
	ResourceId,
	besl::{chain_program, load_pixel_chain},
	handler::{AssetHandler, BakeContext, LoadErrors, SHADER_CODE_SETTING},
};
use crate::{
	asset::BEADType,
	processors::lut::{LutDescription, process_lut},
	resources::lut::{Lut, LutKind},
};

use crate::asset::handler::PrimaryStored;
