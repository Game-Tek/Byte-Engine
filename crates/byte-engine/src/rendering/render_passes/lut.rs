/// The `LutWorkflow` enum selects what a [`LutPass`] does around its 3D LUT: which shader runs and which pass and
/// target names it answers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LutWorkflow {
	/// Applies a creative LUT to scene-linear color and keeps the `main` format, before tone mapping.
	Creative,
	/// Converts scene-linear sRGB to ACEScg and ACEScct before applying the output LUT, then writes SDR display color.
	Aces,
	/// Converts scene-linear sRGB to ACEScg and ACEScct before applying the output LUT, then writes SDR display color
	/// through the ACES 2.0 output transform for a Rec.709, D65, 100-nit display.
	Aces2,
	/// Converts scene-linear sRGB to DaVinci Wide Gamut and DaVinci Intermediate before applying the output LUT, then
	/// writes SDR display color.
	DaVinciWideGamut,
	/// Writes the color a baked table holds for each scene-linear color: a table whose sidecar baked a look through a
	/// shader, such as the ACES 2.0 one. The table already ends in display color.
	Baked,
}

impl LutWorkflow {
	/// Returns the workflow's pipeline, which its pixel step registers with and fusions name it by.
	pub(crate) fn pipeline_id(self) -> &'static str {
		match self {
			Self::Creative => "byte_engine/rendering/lut/apply.pipeline",
			Self::Aces => "byte_engine/rendering/color-grading/aces.pipeline",
			Self::Aces2 => "byte_engine/rendering/color-grading/aces2.pipeline",
			Self::DaVinciWideGamut => "byte_engine/rendering/color-grading/dwg.pipeline",
			Self::Baked => "byte_engine/rendering/color-grading/baked.pipeline",
		}
	}

	/// Returns the stable name that `render.pass.<name>` enables or bypasses.
	fn pass_name(self) -> &'static str {
		match self {
			Self::Creative => "lut",
			Self::Aces => "aces-color-grading",
			Self::Aces2 => "aces2-color-grading",
			Self::DaVinciWideGamut => "dwg-color-grading",
			Self::Baked => "baked-color-grading",
		}
	}

	/// Returns the render-target name of the replacement `main`, which screenshots can capture.
	fn output_name(self) -> &'static str {
		match self {
			Self::Creative => "LUT Output",
			Self::Aces => "ACES Color Grading Output",
			Self::Aces2 => "ACES 2.0 Color Grading Output",
			Self::DaVinciWideGamut => "DWG Color Grading Output",
			Self::Baked => "Baked Color Grading Output",
		}
	}
}

/// The `LutPassSettings` struct lets an application soften a LUT whose look is stronger than the scene needs.
///
/// Pass it to [`crate::application::graphics::setup_aces2_color_grading_render_pass`] or another LUT setup function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LutPassSettings {
	/// How much of the look to apply, from 0 (none) to 1 (all of it). The pass mixes the ungraded and graded color
	/// in the encoding the LUT reads, such as ACEScct, so a half-strength look stays in the LUT's color space.
	pub strength: f32,
}

impl Default for LutPassSettings {
	fn default() -> Self {
		Self { strength: 1.0 }
	}
}

/// The `LutPass` struct applies an asynchronously prepared 3D LUT through renderer-owned GPU storage.
///
/// It is a pixel step: install it through [`crate::application::graphics::setup_lut_render_pass`] or a color-grading
/// setup function, and it fuses with the color correction before it and the tone mapper after it.
pub struct LutPass {
	workflow: LutWorkflow,
	resources: [simple_tile::Resource; 2],
}

/// The `LutGrading` struct registers one LUT workflow with the prepared LUT every sink's [`LutPass`] uploads its own copy
/// of.
///
/// Register it with [`crate::rendering::Renderer::add_pixel_step`] and [`LutWorkflow::pipeline_id`], or with
/// [`crate::application::graphics::setup_lut_render_pass`] or a color-grading setup function.
pub struct LutGrading {
	pub workflow: LutWorkflow,
	pub lut: PreparedLut,
	pub settings: LutPassSettings,
}

impl RenderFeature for LutGrading {
	type Pass = LutPass;

	fn name(&self) -> &'static str {
		self.workflow.pass_name()
	}

	fn create_pass(&mut self, builder: &mut RenderPassBuilder) -> LutPass {
		LutPass::new(builder, self.workflow, self.lut.clone(), self.settings)
	}
}

impl LutPass {
	/// Creates one sink's LUT pass from asynchronously prepared metadata and bytes.
	///
	/// The ACES and ACES 2.0 workflows expect an ACEScct-to-ACEScct LUT. The DaVinci workflow expects a DaVinci Wide
	/// Gamut/Intermediate-to-Intermediate LUT. The baked workflow expects a table baked through a shader by its sidecar,
	/// at a strength of 1. Prepare the LUT through [`PreparedLut::load`] on application-owned
	/// asynchronous work before constructing this render-thread pass.
	pub fn new(
		render_pass_builder: &mut RenderPassBuilder,
		workflow: LutWorkflow,
		lut: PreparedLut,
		settings: LutPassSettings,
	) -> Self {
		let PreparedLut {
			metadata: lut_metadata,
			bytes,
		} = lut;

		assert!(
			matches!(lut_metadata.kind, LutKind::ThreeDimensional),
			"Unsupported LUT kind for LUT render pass. The most likely cause is that the injected LUT resource is not a 3D LUT."
		);
		assert!(
			(0.0..=1.0).contains(&settings.strength),
			"Invalid LUT strength {}. The most likely cause is a strength outside 0 to 1. Set LutPassSettings::strength between 0 (no look) and 1 (the full look).",
			settings.strength
		);
		// A baked table reads scene color and writes display color, so there is no encoding to mix the two in.
		assert!(
			workflow != LutWorkflow::Baked || settings.strength == 1.0,
			"Invalid LUT strength {} for a baked table. The most likely cause is softening a look that was baked with its output transform. Set LutPassSettings::strength to 1, and soften the look in the LUT file before baking.",
			settings.strength
		);

		let queue = render_pass_builder.queue();
		let context = render_pass_builder.context();

		// A linear, clamped sampler reads the LUT.
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		let pixels = lut_rgba16f_texels(&lut_metadata, &bytes);
		let lut_image = create_lookup_texture(
			context,
			queue,
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image)
				.name("LUT Texture")
				.extent(Extent::cube(lut_metadata.size, lut_metadata.size, lut_metadata.size)),
			&pixels,
		);
		let parameters = context.build_buffer_with::<LutParameters>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("LUT Parameters")
				.host_access(ghi::HostAccess::Write),
			|parameters| *parameters = lut_metadata.parameters(settings.strength),
		);

		Self {
			workflow,
			resources: [
				simple_tile::Resource::combined_image_sampler("lut_texture", lut_image, sampler, ghi::Layouts::Read),
				simple_tile::Resource::buffer("lut_parameters", parameters),
			],
		}
	}
}

impl PixelStep<LutGrading> for LutPass {
	fn output(&self) -> &'static str {
		self.workflow.output_name()
	}

	fn output_format(&self, input: ghi::Formats) -> ghi::Formats {
		// A creative LUT grades scene-linear color for later passes; the grading workflows end in display color.
		match self.workflow {
			LutWorkflow::Creative => input,
			LutWorkflow::Aces | LutWorkflow::Aces2 | LutWorkflow::DaVinciWideGamut | LutWorkflow::Baked => {
				crate::rendering::DISPLAY_COLOR_FORMAT
			}
		}
	}

	fn resources(&self) -> &[simple_tile::Resource] {
		&self.resources
	}

	fn prepare(&mut self, _feature: &mut LutGrading, _frame: &mut ghi::implementation::Frame, _sink: &Sink) -> bool {
		true
	}
}

/// Reads the baked LUT payload into owned worker-side bytes.
async fn load_lut_bytes(reference: &mut Reference<Lut>) -> Result<StdBox<[u8]>, String> {
	let read_target = ReadTargetsMut::Box {
		buffer: vec![0_u8; reference.size].into_boxed_slice(),
		offset: 0,
		size: None,
	};
	let read_result = reference.load(read_target).await.map_err(|_| {
		"LUT resource data could not be read. The most likely cause is that the cached payload is missing or unreadable."
			.to_string()
	})?;

	Ok(match read_result {
		resource_management::resource::ReadTargets::Box(bytes) => bytes,
		resource_management::resource::ReadTargets::Buffer(bytes) => bytes.into(),
		resource_management::resource::ReadTargets::Backing(backing) => backing.as_slice().into(),
		resource_management::resource::ReadTargets::Streams(_) => {
			return Err(
				"LUT resource has an unsupported stream layout. The most likely cause is that it was stored as streams instead of a flat payload."
					.to_string(),
			);
		}
	})
}

/// Converts the baked LUT RGB float payload into the compact RGBA16F texels of its 3D texture.
fn lut_rgba16f_texels(lut: &Lut, lut_bytes: &[u8]) -> Vec<u8> {
	assert!(
		matches!(lut.kind, LutKind::ThreeDimensional),
		"Unsupported LUT kind for upload. The most likely cause is that a non-3D LUT resource reached the LUT render pass."
	);
	assert_eq!(
		lut_bytes.len(),
		expected_lut_payload_size(lut),
		"Invalid LUT payload size. The most likely cause is that the baked LUT binary does not match the LUT metadata."
	);

	// The resource stores tightly packed RGB f32 texels, while the GPU texture expects RGBA16F texels.
	lut_bytes
		.as_chunks::<{ 3 * size_of::<f32>() }>()
		.0
		.iter()
		.flat_map(|rgb| {
			let channel = |index: usize| f32::from_le_bytes(rgb[index * 4..][..4].try_into().unwrap());
			[channel(0), channel(1), channel(2), 1.0].map(|value| f16::from_f32(value).to_bits().to_le_bytes())
		})
		.flatten()
		.collect()
}

fn expected_lut_payload_size(lut: &Lut) -> usize {
	lut.kind
		.expected_entry_count(lut.size)
		.and_then(|entry_count| entry_count.checked_mul(3 * std::mem::size_of::<f32>()))
		.expect("Invalid LUT payload size calculation. The most likely cause is that the LUT dimensions overflowed.")
}

/// The `PreparedLut` struct keeps asynchronously loaded LUT metadata and bytes independent from GPU placement.
#[derive(Clone)]
pub struct PreparedLut {
	metadata: Lut,
	bytes: StdBox<[u8]>,
}

impl PreparedLut {
	/// Loads one LUT completely on an application-owned asynchronous task.
	///
	/// Pass the result to [`LutPass::new`] or a LUT setup function on the render
	/// thread. GPU image creation remains owned by the selected render pass.
	pub async fn load(resource_manager: &resource_management::ResourceManager, id: &str) -> Result<Self, String> {
		let mut reference: Reference<Lut> = resource_manager
			.request(id)
			.await
			.map_err(|error| format!("Could not load LUT '{id}'. {error}"))?;
		let metadata = reference.resource().clone();
		let bytes = load_lut_bytes(&mut reference).await?;
		Ok(Self { metadata, bytes })
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, Texture, Value, output_slot};
	use resource_management::resources::lut::{Lut, LutKind};

	use crate::rendering::shader_vm_test::{
		PIXEL_TILE, assert_rgba_close, buffer, compile_pixel_chain, empty_image, push_constant_buffer, rgba, run_pixel, slot,
		texture_2d,
	};

	// The step shaders, as paths under `assets/rendering`.
	const LUT_SHADER: &str = "lut/apply.besl";
	const ACES_SHADER: &str = "color-grading/aces.besl";
	const ACES2_SHADER: &str = "color-grading/aces2.besl";
	const DWG_SHADER: &str = "color-grading/dwg.besl";

	/// A two-point table over the unit cube.
	const UNIT: Lut = Lut {
		kind: LutKind::ThreeDimensional,
		size: 2,
		domain_min: [0.0; 3],
		domain_max: [1.0; 3],
	};

	/// Runs the pixel chain of `steps`, every step applied, over `colors`, one pixel each. The chain reads a two-point
	/// table whose corner `[x, y, z]` holds `corner([x, y, z])`, through the parameters of `lut` at look `strength`.
	fn run_chain(
		steps: &[&str],
		lut: &Lut,
		strength: f32,
		corner: impl Fn([u32; 3]) -> [f32; 3],
		colors: &[[f32; 4]],
	) -> Vec<[f32; 4]> {
		let program = compile_pixel_chain(steps);
		let mut table = Texture::new_3d(2, 2, 2).expect("Expected a valid test LUT extent");
		for index in 0..8 {
			let position = [index % 2, index / 2 % 2, index / 4];
			let [r, g, b] = corner(position);
			table
				.write_3d(position, [r, g, b, 1.0])
				.expect("Expected a valid test LUT coordinate");
		}
		let shader_parameters = lut.parameters(strength);
		let parameter_slot = slot(&program, "lut_parameters");
		let mut parameters = buffer(&program, parameter_slot);
		for (name, value) in [
			("domain_min", shader_parameters.domain_min),
			("domain_scale", shader_parameters.domain_scale),
			("sampling", shader_parameters.sampling),
		] {
			parameters
				.write(name, Value::Vec4F(value))
				.expect("Expected the table's parameters to match the shader");
		}
		// A fused chain applies the steps its mask sets; a lone step takes no mask.
		let mut mask = (steps.len() > 1).then(|| {
			let mut mask = push_constant_buffer(&program);
			mask.write("steps", Value::U32((1 << steps.len()) - 1))
				.expect("Expected a fused chain's step mask");
			mask
		});
		colors
			.iter()
			.map(|color| {
				let mut source = texture_2d(1, 1, &[*color]);
				let mut result = empty_image(1, 1);
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(slot(&program, "source"), &mut source);
				descriptors.bind_texture(slot(&program, "lut_texture"), &mut table);
				descriptors.bind_buffer(parameter_slot, &mut parameters);
				if let Some(mask) = &mut mask {
					descriptors.bind_push_constant(mask);
				}
				descriptors.bind_image(output_slot(0), &mut result);
				run_pixel(&program, &mut descriptors, [0, 0], PIXEL_TILE, [1, 1]);
				drop(descriptors);
				rgba(&result, [0, 0])
			})
			.collect()
	}

	/// A corner that holds its own normalized coordinate, so interpolation reproduces any in-domain color.
	fn identity(position: [u32; 3]) -> [f32; 3] {
		position.map(|axis| axis as f32)
	}

	/// Executes one complete workflow with a two-point identity LUT in its grading encoding.
	fn run_workflow(shader: &str, source_color: [f32; 4]) -> [f32; 4] {
		run_chain(&[shader], &UNIT, 1.0, identity, &[source_color])[0]
	}

	/// The grading workflows keep neutral middle gray through their SDR transforms, and hold black and HDR values to
	/// neutral SDR output.
	#[test]
	fn grading_workflows_preserve_neutral_middle_gray_and_bound_hdr_values() {
		assert_rgba_close(
			run_workflow(ACES_SHADER, [0.18, 0.18, 0.18, 0.4]),
			[0.3584574, 0.3584574, 0.3584574, 0.4],
			4e-4,
		);
		assert_rgba_close(
			run_workflow(DWG_SHADER, [0.18, 0.18, 0.18, 0.4]),
			[0.45925015, 0.45925015, 0.45925015, 0.4],
			4e-4,
		);
		for shader in [ACES_SHADER, ACES2_SHADER, DWG_SHADER] {
			for input in [0.0, 1.0, 16.0] {
				let output = run_workflow(shader, [input, input, input, 0.25]);
				assert!(
					output[..3]
						.iter()
						.all(|channel| channel.is_finite() && (0.0..=1.0).contains(channel)),
					"Invalid fitted SDR output. The most likely cause is unstable grading or display-transform arithmetic: {output:?}"
				);
				assert!((output[0] - output[1]).abs() <= 4e-4 && (output[1] - output[2]).abs() <= 4e-4);
			}
		}
	}

	/// Checks that the bake's lattice step undoes the encoding the baked pass applies, so a table's entry is read back
	/// for the color it was baked for. The chain decodes a lattice position, then the baked pass encodes it again and
	/// reads a table that returns its input.
	#[test]
	fn the_baked_pass_reads_the_entry_its_lattice_position_was_baked_for() {
		// A table over the lattice of a baked table whose corners hold their own positions.
		let lut = Lut {
			domain_min: [0.072_905_534; 3],
			domain_max: [1.467_996_3; 3],
			..UNIT
		};
		let corner = |position: [u32; 3]| position.map(|axis| [lut.domain_min[0], lut.domain_max[0]][axis as usize]);
		// Positions in the encoding's linear toe, around mid grey, and far into the highlights.
		let positions = [
			[0.1, 0.12, 0.14, 0.5],
			[0.4135884, 0.4135884, 0.4135884, 1.0],
			[1.2, 0.3, 0.8, 1.0],
		];
		let steps = ["color-grading/lattice.besl", "color-grading/baked.besl"];
		for (result, position) in run_chain(&steps, &lut, 1.0, corner, &positions).into_iter().zip(positions) {
			assert_rgba_close(result, position, 1e-4);
		}
	}

	/// Checks the ACES 2.0 output transform against the Academy reference, through an identity look.
	///
	/// The expected colors come from `scripts/generate-aces2-tables.py --vectors`, a port of the reference
	/// implementation, which agrees with OpenColorIO 2.6's ACES 2.0 view within 0.001. They cover mid grey at 10 nits,
	/// black, highlights past white, and saturated colors that the chroma and gamut compression reshape.
	#[test]
	fn aces2_grading_matches_the_reference_output_transform() {
		for (input, expected) in [
			([0.18, 0.18, 0.18], [0.3491891, 0.3491891, 0.3491891]),
			([0.0, 0.0, 0.0], [0.0, 0.0, 0.0]),
			([1.0, 1.0, 1.0], [0.7066839, 0.7066839, 0.7066839]),
			([16.0, 16.0, 16.0], [0.9747303, 0.9747303, 0.9747303]),
			([0.5, 0.1, 0.05], [0.5848386, 0.2504861, 0.1614661]),
			([0.05, 0.4, 0.1], [0.1915109, 0.5202122, 0.2732095]),
			([0.02, 0.05, 0.6], [0.0505988, 0.1406792, 0.6100106]),
			([4.0, 0.5, 0.1], [1.0, 0.619354, 0.4811281]),
			([0.3, 0.25, 0.9], [0.4576994, 0.4243927, 0.7209163]),
			([0.9, 0.9, 0.05], [0.6972113, 0.7018813, 0.3009407]),
		] {
			let output = run_workflow(ACES2_SHADER, [input[0], input[1], input[2], 0.5]);
			assert_rgba_close(output, [expected[0], expected[1], expected[2], 0.5], 2e-4);
		}
	}

	/// Verifies identity trilinear interpolation, domain clamping, and alpha preservation, and that the look strength
	/// mixes the input and the looked-up color, through an inverting LUT, through the VM.
	#[test]
	fn lut_besl_vm_trilinearly_applies_the_look_by_its_strength_within_its_domain() {
		let colors = [[0.5, 0.5, 0.5, 0.4], [-1.0, 0.25, 0.75, 0.2], [2.0, 0.75, -2.0, 0.8]];
		let expected = [[0.5, 0.5, 0.5, 0.4], [0.0, 0.25, 0.75, 0.2], [1.0, 0.75, 0.0, 0.8]];
		for (result, expected) in run_chain(&[LUT_SHADER], &UNIT, 1.0, identity, &colors)
			.into_iter()
			.zip(expected)
		{
			assert_rgba_close(result, expected, 1e-6);
		}

		// Each corner stores one minus its coordinate, so the full look maps 0.25 to 0.75.
		let inverting = |position: [u32; 3]| position.map(|axis| 1.0 - axis as f32);
		for (strength, expected) in [(0.0, 0.25), (0.25, 0.375), (1.0, 0.75)] {
			let result = run_chain(&[LUT_SHADER], &UNIT, strength, inverting, &[[0.25, 0.25, 0.25, 0.5]])[0];
			assert_rgba_close(result, [expected, expected, expected, 0.5], 1e-6);
		}
	}
}

use std::boxed::Box as StdBox;

use ghi::context::ContextCreate as _;
use half::f16;
use resource_management::{
	Reference,
	resource::ReadTargetsMut,
	resources::lut::{Lut, LutKind, LutParameters},
};
use utils::Extent;

use crate::rendering::{
	Sink,
	lookup_textures::create_lookup_texture,
	render_pass::{RenderFeature, RenderPassBuilder, pixel_steps::PixelStep, simple_tile},
};
