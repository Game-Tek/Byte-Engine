//! Shared fixtures for executing production rendering shaders through the BESL VM.

use besl::vm::{
	Buffer, DescriptorBindings, ExecutableProgram, ExecutionConfig, ResourceSlot, Texture, TileInvocation, Value, output_slot,
};

const TEST_INSTRUCTION_LIMIT: usize = 4_000_000;
const TEST_CALL_DEPTH_LIMIT: usize = 128;

/// A column-major identity matrix in the BESL VM representation.
pub(crate) const IDENTITY_MATRIX: [f32; 16] = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Converts a row-major matrix to the column-major element order the BESL VM multiplies with.
pub(crate) fn column_major(matrix: maths_rs::Mat4f) -> [f32; 16] {
	std::array::from_fn(|index| matrix[(index % 4) * 4 + index / 4])
}

/// Creates a VM input buffer from a compiled shader interface.
pub(crate) fn input_buffer(program: &ExecutableProgram, index: u8) -> Buffer {
	Buffer::new(program.input_layout(index).expect("Missing VM input layout.").clone())
}

/// Creates a VM output buffer from a compiled shader interface.
pub(crate) fn output_buffer(program: &ExecutableProgram, index: u8) -> Buffer {
	Buffer::new(program.output_layout(index).expect("Missing VM output layout.").clone())
}

/// Creates the builtin-position output buffer from a compiled vertex shader.
pub(crate) fn builtin_position_buffer(program: &ExecutableProgram) -> Buffer {
	Buffer::new(
		program
			.builtin_position_layout()
			.expect("Missing VM position output.")
			.clone(),
	)
}

/// Creates a push-constant buffer from a compiled shader interface.
pub(crate) fn push_constant_buffer(program: &ExecutableProgram) -> Buffer {
	Buffer::new(
		program
			.push_constant_layout()
			.expect("Missing VM push-constant layout.")
			.clone(),
	)
}

/// Reads one checked-in rendering shader. `asset` is its path under `assets/rendering`, such as `bloom/extract.besl`.
///
/// Link the result, alone or with a test's own declarations, through [`link_program`] or [`with_engine_modules`],
/// which attach the modules its `use` declarations name. Each source is read once and kept for the rest of the test
/// run.
pub(crate) fn asset_source(asset: &str) -> &'static str {
	use std::{
		collections::HashMap,
		sync::{LazyLock, Mutex},
	};

	static SOURCES: LazyLock<Mutex<HashMap<String, &'static str>>> = LazyLock::new(Default::default);

	let mut sources = SOURCES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
	if let Some(source) = sources.get(asset) {
		return source;
	}
	let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
		.join("assets/rendering")
		.join(asset);
	let source: &'static str = std::fs::read_to_string(&path)
		.unwrap_or_else(|error| {
			panic!(
				"Failed to read {}: {error}. The most likely cause is a test naming a shader that does not exist.",
				path.display()
			)
		})
		.leak();
	sources.insert(asset.to_string(), source);
	source
}

/// Parses `source`, a checked-in shader or a test's stand-in for one, and attaches the modules from the asset root
/// it names, read from the engine's `assets` directory as `byte_engine/...`, as baking attaches them.
///
/// Next, pass the result to [`besl::lex`]. The modules are kept for the rest of the test run.
pub(crate) fn with_engine_modules(source: &str) -> besl::parser::Node<'static> {
	let source: &'static str = source.to_string().leak();
	let failed = |error: besl::CompilationError| -> ! {
		panic!(
			"Failed to load the modules of a test shader: {error:?}. The most likely cause is a `use` that names a module file that does not exist."
		)
	};
	let read = |file: &str| {
		let file = file.strip_prefix("byte_engine/")?;
		std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets").join(file)).ok()
	};
	let modules: &'static [besl::LoadedModule] =
		Vec::leak(besl::ModuleLoader::load(source, read).unwrap_or_else(|error| failed(error)));
	let mut program = besl::parse(source).unwrap_or_else(|error| {
		panic!("Failed to parse a test shader: {error:?}. The most likely cause is invalid BESL syntax.")
	});
	besl::attach_modules(&mut program, modules).unwrap_or_else(|error| failed(error));
	program
}

/// Links one checked-in BESL shader, with the modules it names, through the frontend production baking uses.
///
/// Returns the program rather than its `main`, because the program owns every function it calls. Pass the result to
/// [`compile`].
pub(crate) fn link_program(source: &str, name: &str) -> besl::NodeReference {
	let program = besl::lex(with_engine_modules(source)).unwrap_or_else(|error| {
		panic!("Failed to link {name}: {error:?}. The most likely cause is invalid syntax in the checked-in BESL asset.")
	});
	program.get_main().unwrap_or_else(|| {
		panic!("Missing {name} entry point. The most likely cause is that the checked-in BESL asset has no `main` function.")
	});
	program
}

/// Links the pixel chain of the checked-in pixel shaders `steps`, each a path under `assets/rendering` such as
/// `lut/apply.besl`, through the generator baking uses, with the engine's common module.
///
/// One step links as a step bakes alone; several link as their fusion bakes, with the step mask in its push constant
/// `steps`. The generated `main` reads `main` through the texture `source` and writes output 0. Pass the result to
/// [`compile`].
pub(crate) fn link_pixel_chain(steps: &[&str]) -> besl::NodeReference {
	use resource_management::shader::pixel_chain::{self, ChainFile};

	let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
	let sources = steps.iter().map(|step| asset_source(step)).collect::<Vec<_>>();
	let read = |file: ChainFile<'_>| match file {
		ChainFile::Root(path) => std::fs::read_to_string(assets.join(path.strip_prefix("byte_engine/")?)).ok(),
		ChainFile::BesideStep(index, path) => {
			std::fs::read_to_string(assets.join("rendering").join(steps[index]).parent()?.join(path)).ok()
		}
		ChainFile::Step(_) | ChainFile::Missing => None,
	};
	let (wrapper, modules) = pixel_chain::assemble(&sources, read)
		.unwrap_or_else(|error| panic!("Failed to generate the pixel chain of {steps:?}: {error}"));
	// The linked program borrows its sources for the rest of the test run.
	let wrapper: &'static str = wrapper.leak();
	let modules: &'static [besl::LoadedModule] = Vec::leak(modules);
	let mut program = pixel_chain::program(wrapper, modules)
		.unwrap_or_else(|error| panic!("Failed to parse the pixel chain of {steps:?}: {error}"));
	program.add(crate::rendering::common_shader_generator::common_shader_module().to_vec());
	besl::lex(program).unwrap_or_else(|error| {
		panic!(
			"Failed to link the pixel chain of {steps:?}: {error:?}. The most likely cause is invalid syntax in a step shader."
		)
	})
}

/// Compiles the exact production shader entry point for a VM runtime test.
pub(crate) fn compile(main: besl::NodeReference) -> ExecutableProgram {
	ExecutableProgram::compile(main).expect(
		"Failed to compile a production shader with the BESL VM. The most likely cause is missing VM support for shader syntax.",
	)
}

/// Creates a tightly initialized two-dimensional texture without intermediate pixel storage.
pub(crate) fn texture_2d(width: u32, height: u32, texels: &[[f32; 4]]) -> Texture {
	assert_eq!(
		texels.len(),
		width as usize * height as usize,
		"Invalid VM texture fixture. The most likely cause is a texel count that does not match its extent."
	);
	let mut texture = Texture::new(width, height)
		.expect("Failed to create a VM texture. The most likely cause is a zero-sized test fixture.");
	for (index, texel) in texels.iter().copied().enumerate() {
		let index = index as u32;
		texture
			.write([index % width, index / width], texel)
			.expect("Failed to initialize a VM texture. The most likely cause is an invalid fixture coordinate.");
	}
	texture
}

/// Creates a zero-initialized image used as a shader output target.
pub(crate) fn empty_image(width: u32, height: u32) -> Texture {
	Texture::new(width, height).expect("Failed to create a VM image. The most likely cause is a zero-sized test fixture.")
}

/// Creates a host buffer using the layout discovered while compiling the shader.
/// Returns the slot of the descriptor a linked program declares as `name`, in the set linking placed it in, so tests
/// bind resources by the name the shader gives them.
pub(crate) fn binding_slot(program: &besl::NodeReference, name: &str) -> ResourceSlot {
	/// Finds the descriptor among the declarations of `scope` and of the scopes and layers nested in it. A struct
	/// member or a local of the same name is not a declaration.
	fn find(scope: &besl::NodeReference, name: &str) -> Option<Option<besl::Placement>> {
		let scope = scope.borrow();
		let (besl::Nodes::Scope { children, .. } | besl::Nodes::Layer { members: children, .. }) = scope.node() else {
			return None;
		};
		children.iter().find_map(|child| match child.borrow().node() {
			besl::Nodes::Binding {
				name: declared,
				placement,
				..
			} if declared == name => Some(*placement),
			besl::Nodes::Scope { .. } | besl::Nodes::Layer { .. } => find(child, name),
			_ => None,
		})
	}

	let placement = find(program, name)
		.unwrap_or_else(|| panic!("Expected the program to declare a descriptor `{name}`"))
		.unwrap_or_else(|| {
			panic!("Expected descriptor `{name}` to belong to a set of the program, through a layer `main` takes")
		});
	ResourceSlot::placed(placement)
}

/// Returns the slot of the descriptor a compiled program names `name`, in the set it was placed in, so tests bind
/// resources by the name the shader gives them. For an array, element `i` is at `slot(program, name).offset(i)`.
pub(crate) fn slot(program: &ExecutableProgram, name: &str) -> ResourceSlot {
	program.slot(name).unwrap_or_else(|| {
		panic!(
			"The program declares no descriptor named `{name}` in its sets. The most likely cause is a misspelled descriptor name or a layer `main` doesn't take. Use a name the shader or one of the layers its `main` takes declares."
		)
	})
}

pub(crate) fn buffer(program: &ExecutableProgram, slot: ResourceSlot) -> Buffer {
	Buffer::new(buffer_layout(program, slot))
}

fn buffer_layout(program: &ExecutableProgram, slot: ResourceSlot) -> besl::vm::BufferLayout {
	program
		.buffer_layout(slot)
		.expect(
			"Missing VM buffer layout. The most likely cause is that the production shader did not retain the expected binding.",
		)
		.clone()
}

/// Creates `element_count` elements of a production runtime-array buffer, such as `vertex_positions: vec3f[]`.
pub(crate) fn array_buffer(program: &ExecutableProgram, slot: ResourceSlot, element_count: usize) -> Buffer {
	Buffer::new_array(buffer_layout(program, slot), element_count).expect(
		"Failed to allocate a VM runtime-array buffer. The most likely cause is an element count that overflows memory.",
	)
}

/// Returns the configuration of workgroup lane `thread_idx`, bounded so a runaway shader fails its test instead of
/// hanging it.
///
/// Workgroup fixtures add the lane's coordinates and pass one per lane to [`ExecutableProgram::run_workgroup`].
pub(crate) fn lane_config(thread_idx: u32) -> ExecutionConfig {
	ExecutionConfig::new(TEST_INSTRUCTION_LIMIT)
		.with_call_depth_limit(TEST_CALL_DEPTH_LIMIT)
		.with_thread_idx(thread_idx)
}

/// The tile of the production tile shaders that run one lane per pixel, as their `.besl.bead` files declare it.
pub(crate) const PIXEL_TILE: u32 = 16;

/// Returns the configurations of the `width` x `width` lanes of tile shader tile `position`, `tile` pixels wide, over
/// outputs of `extent`, in row-major lane order, for [`ExecutableProgram::run_workgroup`].
///
/// Bind each output image at [`besl::vm::output_slot`] of its location.
pub(crate) fn tile_lanes(position: [u32; 2], tile: u32, width: u32, extent: [u32; 2]) -> Vec<ExecutionConfig> {
	(0..width * width)
		.map(|lane| {
			lane_config(lane)
				.with_thread_id([position[0] * width + lane % width, position[1] * width + lane / width])
				.with_tile(TileInvocation {
					size: tile,
					position,
					extent,
				})
		})
		.collect()
}

/// Returns the position of every tile of a `tile`-pixel grid over outputs of `extent`, row by row.
pub(crate) fn tiles(tile: u32, extent: [u32; 2]) -> impl Iterator<Item = [u32; 2]> {
	(0..extent[1].div_ceil(tile)).flat_map(move |y| (0..extent[0].div_ceil(tile)).map(move |x| [x, y]))
}

/// Executes the invocation of a tile shader with one lane per pixel that writes `pixel`, over outputs of `extent` in
/// `tile`-pixel tiles.
pub(crate) fn run_pixel(
	program: &ExecutableProgram,
	descriptors: &mut DescriptorBindings<'_>,
	pixel: [u32; 2],
	tile: u32,
	extent: [u32; 2],
) {
	let config = lane_config((pixel[1] % tile) * tile + pixel[0] % tile)
		.with_thread_id(pixel)
		.with_tile(TileInvocation {
			size: tile,
			position: [pixel[0] / tile, pixel[1] / tile],
			extent,
		});
	program.run_main_with_config(descriptors, &config).expect(
		"Failed to execute a production tile shader with the BESL VM. The most likely cause is missing runtime support or an invalid fixture binding.",
	);
}

/// Executes one bounded shader invocation at the requested two-dimensional thread coordinate.
pub(crate) fn run_at(program: &ExecutableProgram, descriptors: &mut DescriptorBindings<'_>, thread_id: [u32; 2]) {
	program.run_main_with_config(descriptors, &lane_config(0).with_thread_id(thread_id)).expect(
		"Failed to execute a production shader with the BESL VM. The most likely cause is missing runtime support or an invalid fixture binding.",
	);
}

/// Reads one float RGBA texel from a VM texture.
pub(crate) fn rgba(texture: &Texture, coordinate: [u32; 2]) -> [f32; 4] {
	match texture
		.fetch(coordinate)
		.expect("Failed to read a VM texel. The most likely cause is an out-of-bounds assertion coordinate.")
	{
		Value::Vec4F(value) => value,
		_ => panic!("Unexpected VM texel type. The most likely cause is reading an integer image as float RGBA."),
	}
}

/// Compares finite RGBA values component by component with a caller-selected tolerance.
pub(crate) fn assert_rgba_close(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
	for (channel, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
		assert!(
			actual.is_finite() && (actual - expected).abs() <= tolerance,
			"Unexpected VM shader output in channel {channel}: expected {expected}, found {actual}. The most likely cause is a shader regression or incorrect VM semantics."
		);
	}
}

/// The tile of the image-transform shaders, as their `.besl.bead` files declare it.
const IMAGE_TRANSFORM_TILE: u32 = 16;

/// Runs a one-texel pixel chain of steps that take no layer, as [`link_pixel_chain`] links them, and returns the
/// written color.
///
/// Image-transform steps share this: it binds a 1x1 `source` and a 1x1 result to output 0, then reads the single
/// output texel back.
pub(crate) fn run_image_transform_vm(program: &ExecutableProgram, source_color: [f32; 4]) -> [f32; 4] {
	let mut source = texture_2d(1, 1, &[source_color]);
	let mut result = empty_image(1, 1);
	let mut descriptors = DescriptorBindings::new();
	descriptors.bind_texture(slot(program, "source"), &mut source);
	descriptors.bind_image(output_slot(0), &mut result);
	run_pixel(program, &mut descriptors, [0, 0], IMAGE_TRANSFORM_TILE, [1, 1]);
	drop(descriptors);
	rgba(&result, [0, 0])
}
