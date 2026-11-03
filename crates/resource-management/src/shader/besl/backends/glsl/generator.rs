use std::{cell::RefCell, fmt::Write as _};

use super::super::{ResourceAccessorKind, intrinsic_requirements, is_two, resource_accessor, resource_reference_kind};
use crate::shader::generator::{
	NodeEmitter, ShaderFormatting, ShaderGenerationSettings, Stages, emit_statement_block, is_integer_besl_type,
	is_texture_besl_type, ordered_shader_nodes,
};

pub(super) mod mesh_shading;
pub(super) mod ray_tracing;

/// The specialization constant id that holds where the pipeline binds a shader's set 0. Set position `s` reads its
/// pipeline set index from id `SET_SPECIALIZATION_BASE + s`, and defaults to `s`.
///
/// The Vulkan GHI backend fills these constants with the same base, `SET_SPECIALIZATION_BASE` in
/// `ghi/src/vulkan/descriptor_heap.rs`. It sits well above the ids BESL programs give their specializations, and
/// below glslang's limit of 2047.
pub(crate) const SET_SPECIALIZATION_BASE: u32 = 1984;

/// The byte of the push data where a shader's own push constants start, after the sets' base indices.
pub(crate) const PUSH_CONSTANT_OFFSET: u32 = 128;

/// The stride, in bytes, of every resource heap alias. Each descriptor takes one 64-byte slot of the resource heap.
const RESOURCE_DESCRIPTOR_STRIDE: u32 = 64;

/// Names the push data member that holds, for each pipeline set, its resource base index in `.x` and its sampler base
/// index in `.y`.
const SET_BASES: &str = "_besl_sets";

/// Names the sampler heap alias that both application-bound and fixed samplers are read from.
const SAMPLER_HEAP: &str = "_besl_samplers";

/// The `Features` struct holds what a shader's reachable code needs of GLSL beyond the core language, which picks its
/// version, extensions, and prelude.
struct Features {
	f16: bool,
	subgroups: bool,
	ray_queries: bool,
	ray_tracing: bool,
	/// Fragment inputs that a mesh stage writes once per primitive.
	primitive_inputs: bool,
	/// The shader reads a descriptor, so it declares heap aliases and the sets' base indices.
	descriptors: bool,
	/// The shader samples a texture, so it declares the sampler heap alias.
	samplers: bool,
	/// The shader declares push constants, whose block also holds the sets' base indices.
	push_constants: bool,
}

/// The `Generator` struct exists to produce GLSL source for Vulkan-backed shader pipelines.
///
/// # Parameters
///
/// - `minified`: Controls compact shader output. The default is `true` in release builds.
pub struct Generator {
	minified: bool,
	/// The stage being generated, which decides interpolation qualifiers and workgroup storage support.
	stage: Stages,
	match_break_depth: Option<usize>,
	/// The program's `task_payload` arrays, in declaration order, which become the members of one payload struct.
	task_payloads: Vec<besl::NodeReference>,
	/// One bit per set position whose pipeline index constant is already declared.
	declared_sets: u32,
	/// One bit per texture type whose resource heap alias texture parameters read is already declared.
	declared_parameter_textures: u8,
}

impl Generator {
	/// Creates a GLSL transpiler with the default formatting mode.
	pub fn new() -> Self {
		Generator {
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			stage: Stages::Vertex,
			match_break_depth: None,
			task_payloads: Vec::new(),
			declared_sets: 0,
			declared_parameter_textures: 0,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	/// Generates a GLSL shader from a BESL AST.
	///
	/// # Arguments
	///
	/// * `shader_compilation_settings` - The shader compilation settings.
	/// * `main_function_node` - The shader's main function node.
	///
	/// # Returns
	///
	/// The GLSL shader as a string.
	///
	/// # Panics
	///
	/// Panics if the main function node is not a function node.
	pub fn generate(
		&mut self,
		shader_compilation_settings: &ShaderGenerationSettings,
		main_function_node: &besl::NodeReference,
	) -> Result<String, ()> {
		self.stage = shader_compilation_settings.stage;
		self.declared_sets = 0;
		self.declared_parameter_textures = 0;
		let mut string = String::with_capacity(2048);
		let mut order = ordered_shader_nodes(main_function_node, "GLSL");
		// Every reachable descriptor is addressed through its set's base indices, so it needs a placement in a set the
		// push data holds bases for.
		let mut descriptors = false;
		let mut samplers = false;
		for node in &order {
			if let besl::Nodes::Binding { placement, r#type, .. } = node.borrow().node() {
				if placement.is_none_or(|placement| placement.set >= besl::MAX_DESCRIPTOR_SETS) {
					return Err(());
				}
				descriptors = true;
				samplers |= matches!(r#type, besl::BindingTypes::CombinedImageSampler { .. });
			}
		}
		let push_constants = hoist_push_constant(&mut order);
		crate::shader::generator::validate_workgroup_storage_stage(&shader_compilation_settings.stage, &order)?;
		self.task_payloads = order
			.iter()
			.filter(|node| matches!(node.borrow().node(), besl::Nodes::TaskPayload { .. }))
			.cloned()
			.collect();
		crate::shader::generator::validate_vertex_builtin_inputs(&shader_compilation_settings.stage, &order)?;
		let requirements = intrinsic_requirements(&order);
		let ray_queries = Self::uses_ray_queries(&order, &requirements);
		let features = Features {
			// Reachable code needs native 16-bit floating-point arithmetic when it declares or converts to an f16 type.
			f16: requirements.uses_f16
				|| order.iter().any(|node| {
					matches!(node.borrow().node(), besl::Nodes::Struct { name, .. } if matches!(name.as_str(), "f16" | "vec2f16" | "vec3f16" | "vec4f16"))
				}),
			subgroups: requirements.uses_subgroup_intrinsics,
			ray_queries,
			ray_tracing: Self::uses_ray_tracing(self.stage, &order, ray_queries),
			primitive_inputs: order
				.iter()
				.any(|node| matches!(node.borrow().node(), besl::Nodes::Input { per_primitive: true, .. })),
			descriptors,
			samplers,
			push_constants,
		};
		if features.subgroups && !matches!(shader_compilation_settings.stage, Stages::Compute { .. }) {
			return Err(());
		}

		self.generate_glsl_header_block(&mut string, shader_compilation_settings, &features);
		if features.ray_tracing {
			self.emit_ray_tracing_prelude(&mut string, &requirements);
		}

		// Ray tracing stages exchange the payload as callable data, which is declared right after the payload's struct.
		let payload = match self.stage {
			Stages::RayGeneration => super::super::invoke_payload(&order),
			Stages::ClosestHit => match main_function_node.borrow().node() {
				besl::Nodes::Function { return_type, .. } => Some(return_type.clone()),
				_ => None,
			},
			_ => None,
		};
		let payload = payload.and_then(|payload| payload.borrow().get_name().map(str::to_owned));
		for node in order {
			if node == *main_function_node {
				self.emit_entry_point(&mut string, &node)?;
				continue;
			}
			self.emit_node(&mut string, &node);
			if let besl::Nodes::Struct { name, .. } = node.borrow().node()
				&& payload.as_deref() == Some(name.as_str())
			{
				self.emit_call_data(&mut string, name);
			}
		}

		Ok(string)
	}

	/// Emits `main`, as GLSL's `main` for most stages. A closest hit and a task stage run it as a function of their
	/// own, which their entry point calls.
	fn emit_entry_point(&mut self, string: &mut String, main: &besl::NodeReference) -> Result<(), ()> {
		match self.stage {
			Stages::ClosestHit => self.emit_closest_hit_entry_point(string, main),
			Stages::Task { .. } => self.emit_task_entry_point(string, main),
			_ => {
				self.emit_node(string, main);
				Ok(())
			}
		}
	}

	/// Emits the function `main` under `name`, after checking it takes `parameters` parameters. The backend owns the
	/// name, so it is written as it is rather than escaped like a user name.
	fn emit_main_as(
		&mut self,
		string: &mut String,
		main: &besl::NodeReference,
		name: &str,
		parameters: usize,
	) -> Result<(), ()> {
		let node = main.borrow();
		let besl::Nodes::Function {
			params,
			return_type,
			statements,
			..
		} = node.node()
		else {
			return Err(());
		};
		if params.len() != parameters {
			return Err(());
		}
		Self::emit_type_name(string, return_type.borrow().get_name().ok_or(())?);
		let _ = write!(string, " {name}(");
		self.emit_function_parameters(string, params);
		ShaderFormatting::new(self.minified).push_block_start(string);
		self.emit_function_statement_block(string, statements, 1);
		self.emit_block_end(string);
		Ok(())
	}

	/// Emits the GLSL version, stage, extension, and layout declarations.
	fn generate_glsl_header_block(
		&self,
		glsl_block: &mut String,
		compilation_settings: &ShaderGenerationSettings,
		features: &Features,
	) {
		// Ray tracing and ray queries need GLSL 4.60. Other shaders keep 4.50, so their SPIR-V stays as it was.
		glsl_block.push_str(if features.ray_tracing {
			"#version 460 core\n"
		} else {
			"#version 450 core\n"
		});

		match compilation_settings.stage {
			Stages::Vertex => glsl_block.push_str("#pragma shader_stage(vertex)\n"),
			Stages::Fragment => glsl_block.push_str("#pragma shader_stage(fragment)\n"),
			Stages::Compute { .. } => glsl_block.push_str("#pragma shader_stage(compute)\n"),
			Stages::Tile { .. } => {
				unreachable!(
					"A tile shader reaches GLSL already rewritten into a compute shader by `backends::tile::lower_for`."
				)
			}
			Stages::Task { .. } => glsl_block.push_str("#pragma shader_stage(task)\n"),
			Stages::Mesh { .. } => glsl_block.push_str("#pragma shader_stage(mesh)\n"),
			Stages::RayGeneration => glsl_block.push_str("#pragma shader_stage(raygen)\n"),
			// The ray generation shader calls a hit's closest hit through the shader binding table, so it is callable.
			Stages::ClosestHit => glsl_block.push_str("#pragma shader_stage(callable)\n"),
		}

		glsl_block.push_str("#extension GL_EXT_shader_16bit_storage:require\n");
		glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types:require\n");
		if features.f16 {
			glsl_block.push_str("#extension GL_EXT_shader_explicit_arithmetic_types_float16:require\n");
		}
		glsl_block.push_str("#extension GL_EXT_nonuniform_qualifier:require\n");
		glsl_block.push_str("#extension GL_EXT_scalar_block_layout:require\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference:enable\n");
		glsl_block.push_str("#extension GL_EXT_buffer_reference2:enable\n");
		glsl_block.push_str("#extension GL_EXT_shader_image_load_formatted:enable\n");
		if features.descriptors {
			glsl_block.push_str("#extension GL_EXT_descriptor_heap:require\n");
		}
		// Read-only images are textures without a sampler, which `texelFetch` and `textureSize` read through this.
		glsl_block.push_str("#extension GL_EXT_samplerless_texture_functions:enable\n");
		if compilation_settings.stage.is_ray_tracing() {
			glsl_block.push_str("#extension GL_EXT_ray_tracing:require\n");
		}
		if features.ray_queries {
			glsl_block.push_str("#extension GL_EXT_ray_query:require\n");
		}
		// Inputs that a mesh stage writes once per primitive need the extension even in a fragment shader.
		if matches!(compilation_settings.stage, Stages::Task { .. } | Stages::Mesh { .. }) || features.primitive_inputs {
			glsl_block.push_str("#extension GL_EXT_mesh_shader:require\n");
		}

		match compilation_settings.stage {
			Stages::Compute { .. } if features.subgroups => {
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_basic:require\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_ballot:require\n");
				glsl_block.push_str("#extension GL_KHR_shader_subgroup_shuffle:require\n");
			}
			Stages::Mesh {
				maximum_vertices,
				maximum_primitives,
				..
			} => {
				let _ = writeln!(
					glsl_block,
					"layout(triangles,max_vertices={maximum_vertices},max_primitives={maximum_primitives}) out;"
				);
			}
			_ => {}
		}

		if let Some(local_size) = compilation_settings.stage.local_size() {
			let _ = writeln!(
				glsl_block,
				"layout(local_size_x={},local_size_y={},local_size_z={}) in;",
				local_size.width(),
				local_size.height(),
				local_size.depth()
			);
		}
		// `set_task_mesh_output_count` stores the grid size here for the task entry point, from any function.
		if matches!(compilation_settings.stage, Stages::Task { .. }) {
			let _ = writeln!(glsl_block, "shared uint {};", mesh_shading::TASK_MESH_COUNT);
		}

		// BESL matrices are always row major in uniform and storage buffers.
		glsl_block.push_str("layout(row_major) uniform;layout(row_major) buffer;\n");

		glsl_block.push_str("const float PI = 3.14159265359;");
		glsl_block.push_str(
			"bool _besl_is_finite(float value){return !isnan(value)&&!isinf(value);}\n\
			 bool _besl_is_normal(float value){return _besl_is_finite(value)&&abs(value)>=1.1754943508222875e-38;}\n",
		);
		if features.f16 {
			glsl_block.push_str(
				"bool _besl_is_finite(float16_t value){return !isnan(value)&&!isinf(value);}\n\
				 bool _besl_is_normal(float16_t value){return _besl_is_finite(value)&&abs(value)>=float16_t(0.00006103515625);}\n",
			);
		}
		let break_str = ShaderFormatting::new(self.minified).break_str();
		// A shader with push constants declares the sets' base indices in their block, where its `PushConstant` node is.
		if features.descriptors && !features.push_constants {
			let _ = write!(
				glsl_block,
				"layout(push_constant,scalar)uniform PushConstant{{uvec2 {SET_BASES}[{}];}}push_constant;{break_str}",
				besl::MAX_DESCRIPTOR_SETS
			);
		}
		// Fixed samplers and the ones the application binds share the sampler heap, at the device's sampler stride.
		if features.samplers {
			let _ = write!(
				glsl_block,
				"layout(descriptor_heap)uniform sampler {SAMPLER_HEAP}[];{break_str}"
			);
		}
		glsl_block.push_str(break_str);
	}

	/// Declares the specialization constant that holds the pipeline index of the shader's set `set`, once per shader.
	fn declare_set(&mut self, string: &mut String, set: u32) {
		if self.declared_sets & (1 << set) != 0 {
			return;
		}
		self.declared_sets |= 1 << set;
		let _ = write!(
			string,
			"layout(constant_id={})const uint _besl_set{set}={set}u;{}",
			SET_SPECIALIZATION_BASE + set,
			ShaderFormatting::new(self.minified).break_str()
		);
	}

	/// Writes the heap index of descriptor `offset` of the shader's set `set`, plus `index` when one element of an array
	/// is picked. `component` is `x` for the resource heap and `y` for the sampler heap.
	///
	/// An index any expression picks may differ across a draw, so it is marked non-uniform.
	fn emit_heap_index(
		&mut self,
		string: &mut String,
		set: u32,
		component: char,
		offset: u32,
		index: Option<&besl::NodeReference>,
	) {
		if index.is_some() {
			string.push_str("nonuniformEXT(");
		}
		let _ = write!(string, "push_constant.{SET_BASES}[_besl_set{set}].{component}+{offset}u");
		if let Some(index) = index {
			string.push_str("+(");
			self.emit_node(string, index);
			string.push_str("))");
		}
	}

	/// Writes the descriptor `binding` names, or element `index` of it when it is an array, read from the heap aliases
	/// its declaration made. A texture the shader samples is combined with its sampler, as GLSL's texture functions
	/// take it.
	fn emit_binding_access(&mut self, string: &mut String, binding: &besl::NodeReference, index: Option<&besl::NodeReference>) {
		let node = binding.borrow();
		let besl::Nodes::Binding { name, r#type, .. } = node.node() else {
			unreachable!("Expected a binding node.");
		};
		match r#type {
			besl::BindingTypes::CombinedImageSampler { format, .. } => {
				string.push_str(combined_sampler_type(format));
				string.push('(');
				self.emit_texture_and_sampler(string, binding, index);
				string.push(')');
			}
			besl::BindingTypes::BufferArray { .. } => {
				self.emit_resource_element(string, binding, index);
				string.push('.');
				Self::identifier(name).push_to(string);
			}
			_ => self.emit_resource_element(string, binding, index),
		}
	}

	/// Writes the resource heap element of `binding`, or of its element `index`.
	fn emit_resource_element(
		&mut self,
		string: &mut String,
		binding: &besl::NodeReference,
		index: Option<&besl::NodeReference>,
	) {
		let node = binding.borrow();
		let besl::Nodes::Binding {
			name,
			placement: Some(placement),
			..
		} = node.node()
		else {
			unreachable!("`generate` admits only placed bindings.");
		};
		Self::identifier(name).push_to(string);
		string.push('[');
		self.emit_heap_index(string, placement.set, 'x', placement.resource, index);
		string.push(']');
	}

	/// Writes the texture of the texture binding `binding`, or of its element `index`, then a comma and its sampler:
	/// the one the application binds in the set, or the fixed one at the sampler heap index its state decides.
	fn emit_texture_and_sampler(&mut self, string: &mut String, binding: &besl::NodeReference, index: Option<&besl::NodeReference>) {
		self.emit_resource_element(string, binding, index);
		string.push(',');
		string.push_str(SAMPLER_HEAP);
		string.push('[');
		self.emit_sampler_index(string, binding, index);
		string.push(']');
	}

	/// Writes the heap indices of the texture binding `binding`, or of its element `index`: its texture's resource heap
	/// index, a comma, and its sampler's sampler heap index.
	fn emit_texture_and_sampler_indices(
		&mut self,
		string: &mut String,
		binding: &besl::NodeReference,
		index: Option<&besl::NodeReference>,
	) {
		let placement = match binding.borrow().node() {
			besl::Nodes::Binding {
				placement: Some(placement),
				..
			} => *placement,
			_ => unreachable!("`generate` admits only placed bindings."),
		};
		self.emit_heap_index(string, placement.set, 'x', placement.resource, index);
		string.push(',');
		self.emit_sampler_index(string, binding, index);
	}

	/// Writes the sampler heap index of the sampler of the texture binding `binding`, or of its element `index`.
	fn emit_sampler_index(&mut self, string: &mut String, binding: &besl::NodeReference, index: Option<&besl::NodeReference>) {
		let node = binding.borrow();
		let besl::Nodes::Binding {
			r#type: besl::BindingTypes::CombinedImageSampler { sampler, .. },
			placement: Some(placement),
			..
		} = node.node()
		else {
			unreachable!("Expected a placed texture binding.");
		};
		match sampler {
			Some(sampler) => {
				let _ = write!(string, "{}u", sampler.fixed_index());
			}
			None => self.emit_heap_index(string, placement.set, 'y', placement.sampler, index),
		}
	}

	/// Declares, once per shader, the resource heap alias that a texture parameter of BESL type `type_name` reads.
	fn declare_parameter_texture(&mut self, string: &mut String, type_name: &str) {
		let texture_type = parameter_texture_type(type_name);
		let bit = match texture_type {
			"texture3D" => 1,
			"textureCube" => 2,
			"textureCubeArray" => 4,
			"texture2DArray" => 8,
			_ => 16,
		};
		if self.declared_parameter_textures & bit != 0 {
			return;
		}
		self.declared_parameter_textures |= bit;
		let _ = write!(
			string,
			"layout(descriptor_heap,descriptor_stride={RESOURCE_DESCRIPTOR_STRIDE}) uniform {texture_type} _besl_{texture_type}[];{}",
			ShaderFormatting::new(self.minified).break_str()
		);
	}

	/// Writes a texture argument of a user function as the resource heap index of the texture, a comma, and the
	/// sampler heap index of its sampler, which the parameter pair takes.
	fn emit_texture_argument(&mut self, string: &mut String, argument: &besl::NodeReference) {
		// A descriptor array element names its binding through an accessor.
		let (binding, index) = match argument.borrow().node() {
			besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) => (direct_binding(left), Some(right.clone())),
			_ => (direct_binding(argument), None),
		};
		if let Some(binding) = binding
			&& matches!(
				binding.borrow().node(),
				besl::Nodes::Binding {
					r#type: besl::BindingTypes::CombinedImageSampler { .. },
					..
				}
			) {
			self.emit_texture_and_sampler_indices(string, &binding, index.as_ref());
			return;
		}
		// A texture parameter passes on with the sampler it came with.
		if let Some(name) = texture_parameter(argument) {
			let name = Self::identifier(&name);
			let _ = write!(string, "{name},{name}_sampler");
			return;
		}
		self.emit_node(string, argument);
	}

	// Emits ordinary 2D samples, descriptor-array samples, and one selected 2D-array layer.
	fn emit_sample(&mut self, string: &mut String, arguments: &[besl::NodeReference]) {
		string.push_str("texture(");
		let accessor = resource_accessor(&arguments[0]);
		if let Some((ResourceAccessorKind::Texture2DArrayLayer, resource, _)) = &accessor {
			self.emit_node(string, resource);
		} else {
			self.emit_node(string, &arguments[0]);
		}
		self.emit_separator(string);
		if let Some((ResourceAccessorKind::Texture2DArrayLayer, _, layer)) = accessor {
			string.push_str("vec3(");
			self.emit_node(string, &arguments[1]);
			self.emit_separator(string);
			string.push_str("float(");
			self.emit_node(string, &layer);
			string.push_str("))");
		} else {
			self.emit_node(string, &arguments[1]);
		}
		string.push(')');
	}

	/// Writes the extent of `image` as an `ivec2`: `textureSize` for a `sampled` texture and `imageSize` for a storage
	/// image.
	fn emit_image_size(&mut self, string: &mut String, image: &besl::NodeReference, sampled: bool) {
		string.push_str(if sampled { "textureSize(" } else { "imageSize(" });
		self.emit_node(string, image);
		string.push_str(if sampled { ",0)" } else { ")" });
	}
}

/// Reports whether `image`, an image intrinsic's argument, names a sampled image or one element of an array of them.
/// GLSL declares those as textures without a sampler, which `texelFetch` and `textureSize` read.
fn reads_sampled_image(image: &besl::NodeReference) -> bool {
	besl::referenced_binding(image).is_some_and(|binding| {
		matches!(binding.borrow().node(), besl::Nodes::Binding { r#type, read, write, .. } if r#type.is_sampled_image(*read, *write))
	})
}

/// Returns the binding `node` names directly, as a reference to it, without an accessor in between.
fn direct_binding(node: &besl::NodeReference) -> Option<besl::NodeReference> {
	match node.borrow().node() {
		besl::Nodes::Binding { .. } => Some(node.clone()),
		besl::Nodes::Expression(besl::Expressions::Member { source, .. })
			if matches!(source.borrow().node(), besl::Nodes::Binding { .. }) =>
		{
			Some(source.clone())
		}
		_ => None,
	}
}

/// Returns the name of the texture parameter `node` names, as a reference to it.
fn texture_parameter(node: &besl::NodeReference) -> Option<String> {
	let node = node.borrow();
	let besl::Nodes::Expression(besl::Expressions::Member { source, .. }) = node.node() else {
		return None;
	};
	let source = source.borrow();
	let besl::Nodes::Parameter { name, r#type } = source.node() else {
		return None;
	};
	r#type
		.borrow()
		.get_name()
		.is_some_and(is_texture_besl_type)
		.then(|| name.clone())
}

/// Moves the push constant block, and the structs its members name, to the front of `order`, and returns whether the
/// program has one.
///
/// The block holds the sets' base indices that every descriptor access reads, so it has to be declared before any
/// function that reads a descriptor. `order` lists dependencies first, so moving a dependency-closed group to the front
/// keeps every node after what it names.
fn hoist_push_constant(order: &mut Vec<besl::NodeReference>) -> bool {
	/// Appends `node`'s struct dependencies, then `node`, to `group`, once each.
	fn collect(node: &besl::NodeReference, group: &mut Vec<besl::NodeReference>) {
		if group.contains(node) {
			return;
		}
		match node.borrow().node() {
			besl::Nodes::PushConstant { members: children } | besl::Nodes::Struct { fields: children, .. } => {
				children.iter().for_each(|child| collect(child, group))
			}
			besl::Nodes::Member { r#type, .. } => collect(r#type, group),
			_ => {}
		}
		group.push(node.clone());
	}

	let Some(push_constant) = order
		.iter()
		.find(|node| matches!(node.borrow().node(), besl::Nodes::PushConstant { .. }))
		.cloned()
	else {
		return false;
	};
	let mut group = Vec::new();
	collect(&push_constant, &mut group);
	// Only emitted nodes move; members and built-in types are not in `order`.
	group.retain(|node| order.contains(node));
	order.retain(|node| !group.contains(node));
	group.append(order);
	*order = group;
	true
}

/// Returns how a texture of BESL format `format` is declared in the resource heap: as a texture without its sampler.
fn texture_declaration(format: &str) -> &'static str {
	match format {
		"Texture3D" => "uniform texture3D",
		"TextureCube" => "uniform textureCube",
		"TextureCubeArray" => "uniform textureCubeArray",
		"ArrayTexture2D" => "uniform texture2DArray",
		"r8ui" | "r16ui" | "r32ui" => "uniform utexture2D",
		_ => "uniform texture2D",
	}
}

/// Returns the GLSL texture type that a texture parameter of BESL type `type_name` reads its texture as.
fn parameter_texture_type(type_name: &str) -> &'static str {
	match type_name {
		"Texture3D" => "texture3D",
		"TextureCube" => "textureCube",
		"TextureCubeArray" => "textureCubeArray",
		"ArrayTexture2D" => "texture2DArray",
		_ => "texture2D",
	}
}

/// Returns the GLSL sampler type that combines a texture of BESL format `format` with a sampler.
fn combined_sampler_type(format: &str) -> &'static str {
	match format {
		"Texture3D" => "sampler3D",
		"TextureCube" => "samplerCube",
		"TextureCubeArray" => "samplerCubeArray",
		"ArrayTexture2D" => "sampler2DArray",
		"r8ui" | "r16ui" | "r32ui" => "usampler2D",
		_ => "sampler2D",
	}
}

impl NodeEmitter for Generator {
	/// Translates BESL intrinsic type names to GLSL type names, such as `vec2f` to `vec2`.
	fn type_from_besl(source: &str) -> &str {
		match source {
			"void" => "void",
			"atomicu32" => "uint32_t",
			"atomici32" => "int32_t",
			"vec2f16" => "f16vec2",
			"vec3f16" => "f16vec3",
			"vec4f16" => "f16vec4",
			"vec2f" => "vec2",
			"vec2u" => "uvec2",
			"vec2i" => "ivec2",
			"vec2u16" => "u16vec2",
			"vec3u16" => "u16vec3",
			"vec4u16" => "u16vec4",
			"vec3u" => "uvec3",
			"vec4u" => "uvec4",
			"vec3f" => "vec3",
			"vec4f" => "vec4",
			"mat2f" => "mat2",
			"mat3f" => "mat3",
			"mat4f" => "mat4",
			"mat4x3f" => "mat4x3",
			"f16" => "float16_t",
			"f32" => "float",
			"u8" => "uint8_t",
			"u16" => "uint16_t",
			"u32" => "uint32_t",
			"i32" => "int32_t",
			// A texture parameter takes the texture's resource heap index, followed by its sampler's sampler heap index;
			// see `emit_texture_parameter_sampler`. glslang can't pass a heap alias's element to a function.
			"Texture2D" | "Texture3D" | "TextureCube" | "TextureCubeArray" | "ArrayTexture2D" => "uint",
			_ => source,
		}
	}
	const SPECIALIZATION_QUALIFIER: &'static str = "const";
	fn emit_specialization_constant(
		&self,
		string: &mut String,
		type_name: &str,
		name: std::fmt::Arguments<'_>,
		id: u32,
		default: &str,
	) {
		let _ = write!(string, "layout(constant_id={id})const {type_name} {name}={default};");
	}
	fn minified(&self) -> bool {
		self.minified
	}
	fn match_break_depth(&mut self) -> &mut Option<usize> {
		&mut self.match_break_depth
	}
	fn is_reserved_identifier(name: &str) -> bool {
		super::reserved::is_reserved(name)
	}
	// Keep the intrinsic table contiguous so unsupported names cannot silently drift between GLSL call forms.
	#[allow(clippy::too_many_lines)]
	fn emit_intrinsic_call(
		&mut self,
		string: &mut String,
		intrinsic: &besl::NodeReference,
		arguments: &[besl::NodeReference],
		elements: &[besl::NodeReference],
	) {
		let intrinsic = intrinsic.borrow();
		let besl::Nodes::Intrinsic {
			name,
			elements: definition,
			r#return,
		} = intrinsic.node()
		else {
			for element in elements {
				self.emit_node(string, element);
			}
			return;
		};

		let has_body = definition
			.iter()
			.any(|element| !matches!(element.borrow().node(), besl::Nodes::Parameter { .. }));
		match name.as_str() {
			// Ray tracing stage values are built-in variables, or members of the hit a closest hit shades. See
			// `ray_tracing.rs`.
			"launch_index" => string.push_str("gl_LaunchIDEXT"),
			"launch_size" => string.push_str("gl_LaunchSizeEXT"),
			"hit_distance"
			| "hit_custom_index"
			| "hit_geometry_index"
			| "hit_primitive_index"
			| "hit_barycentrics"
			| "ray_origin"
			| "ray_direction" => Self::emit_hit_value(string, name),
			// These call the helpers the ray tracing prelude and the callable data declare.
			"intersect" | "trace_occlusion" | "invoke" => {
				let _ = write!(string, "_besl_{name}(");
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
			// Texture lowerings bypass intrinsic bodies.
			"sample" => self.emit_sample(string, arguments),
			"sample_texture_2d_array_grad" => {
				string.push_str("textureGrad(");
				let Some(textures) = direct_binding(&arguments[0]) else {
					unreachable!("`sample_texture_2d_array_grad` takes an array of textures as its first argument.");
				};
				self.emit_binding_access(string, &textures, Some(&arguments[1]));
				for argument in &arguments[2..5] {
					self.emit_separator(string);
					self.emit_node(string, argument);
				}
				string.push(')');
			}
			"gather" => {
				string.push_str("textureGather(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				if let Some(layer) = arguments.get(2) {
					string.push_str("vec3(");
					self.emit_node(string, &arguments[1]);
					self.emit_separator(string);
					string.push_str("float(");
					self.emit_node(string, layer);
					string.push_str("))");
				} else {
					self.emit_node(string, &arguments[1]);
				}
				string.push(')');
			}
			"texture_lod" | "downsample_min" | "downsample_max" => {
				string.push_str("textureLod(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				if arguments.len() == 4 {
					string.push_str("vec3(");
					self.emit_node(string, &arguments[1]);
					self.emit_separator(string);
					string.push_str("float(");
					self.emit_node(string, &arguments[2]);
					string.push_str("))");
				} else {
					self.emit_node(string, &arguments[1]);
				}
				self.emit_separator(string);
				if let Some(lod) = arguments.get(if arguments.len() == 4 { 3 } else { 2 }) {
					self.emit_node(string, lod);
				} else {
					string.push_str("0.0");
				}
				string.push(')');
				if name != "texture_lod" {
					string.push_str(".x");
				}
			}
			"texture_cube_array_lod" => {
				string.push_str("textureLod(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("vec4(");
				self.emit_node(string, &arguments[1]);
				self.emit_separator(string);
				string.push_str("float(");
				self.emit_node(string, &arguments[2]);
				string.push_str("))");
				self.emit_separator(string);
				self.emit_node(string, &arguments[3]);
				string.push(')');
			}
			// Every other intrinsic with a body emits its expansion.
			_ if has_body => {
				for element in elements {
					self.emit_node(string, element);
				}
			}
			// A sampled image is a texture without a sampler, which `texelFetch` reads like a fetch.
			"fetch_u32" | "image_load_u32" if name == "fetch_u32" || reads_sampled_image(&arguments[0]) => {
				string.push_str("texelFetch(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("ivec2(");
				self.emit_node(string, &arguments[1]);
				string.push_str("),0).x");
			}
			"fetch" | "image_load" if name == "fetch" || reads_sampled_image(&arguments[0]) => {
				string.push_str("texelFetch(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				if arguments.len() == 3 {
					string.push_str("ivec3(ivec2(");
				} else {
					string.push_str("ivec2(");
				}
				self.emit_node(string, &arguments[1]);
				if let Some(layer) = arguments.get(2) {
					string.push_str("),int(");
					self.emit_node(string, layer);
					string.push_str(")),0)");
				} else {
					string.push_str("),0)");
				}
			}
			"texture_size" | "image_size" => {
				string.push_str("uvec2(");
				let sampled = name == "texture_size" || reads_sampled_image(&arguments[0]);
				self.emit_image_size(string, &arguments[0], sampled);
				string.push(')');
			}
			"image_load" | "image_load_u32" => {
				string.push_str("imageLoad(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("ivec2(");
				self.emit_node(string, &arguments[1]);
				string.push_str(if name == "image_load_u32" { ")).x" } else { "))" });
			}
			"write" | "image_atomic_or" => {
				string.push_str(if name == "write" { "imageStore(" } else { "imageAtomicOr(" });
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("ivec2(");
				self.emit_node(string, &arguments[1]);
				string.push(')');
				self.emit_separator(string);
				self.emit_node(string, &arguments[2]);
				string.push(')');
			}
			"guard_image_bounds" => {
				let sampled = reads_sampled_image(&arguments[0]);
				string.push_str("if(");
				self.emit_node(string, &arguments[1]);
				string.push_str(".x>=uint(");
				self.emit_image_size(string, &arguments[0], sampled);
				string.push_str(".x)||");
				self.emit_node(string, &arguments[1]);
				string.push_str(".y>=uint(");
				self.emit_image_size(string, &arguments[0], sampled);
				string.push_str(".y)){return;}");
			}
			"pow" if arguments.len() == 2 && is_two(&arguments[0]) => {
				string.push_str("exp2(");
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			// findLSB returns -1 for zero, which converts to BESL's 0xFFFFFFFF.
			"find_lsb" => {
				string.push_str("uint(findLSB(");
				self.emit_call_arguments(string, arguments);
				string.push_str("))");
			}
			"sincos" => {
				string.push_str("vec2(sin(");
				self.emit_node(string, &arguments[0]);
				string.push_str("), cos(");
				self.emit_node(string, &arguments[0]);
				string.push_str("))");
			}
			"round_to_i32" => {
				string.push_str("ivec2(round(");
				self.emit_node(string, &arguments[0]);
				string.push_str("))");
			}
			"atomic_exchange" | "atomic_add" | "atomic_min" | "atomic_max" | "atomic_and" | "atomic_or" | "atomic_xor" => {
				string.push_str(match name.as_str() {
					"atomic_exchange" => "atomicExchange(",
					"atomic_add" => "atomicAdd(",
					"atomic_min" => "atomicMin(",
					"atomic_max" => "atomicMax(",
					"atomic_and" => "atomicAnd(",
					"atomic_or" => "atomicOr(",
					"atomic_xor" => "atomicXor(",
					_ => unreachable!("Expected an atomic binary intrinsic"),
				});
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			"atomic_sub" => {
				string.push_str("atomicAdd(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str("-(");
				self.emit_node(string, &arguments[1]);
				string.push_str("))");
			}
			"atomic_compare_exchange" => {
				string.push_str("atomicCompSwap(");
				self.emit_node(string, &arguments[0]);
				for argument in &arguments[1..] {
					self.emit_separator(string);
					self.emit_node(string, argument);
				}
				string.push(')');
			}
			"atomic_load" => {
				string.push_str("atomicAdd(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				string.push_str(if r#return.borrow().get_name() == Some("i32") {
					"0"
				} else {
					"0u"
				});
				string.push(')');
			}
			"atomic_store" => {
				string.push_str("atomicExchange(");
				self.emit_node(string, &arguments[0]);
				self.emit_separator(string);
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			"thread_id" => string.push_str("uvec2(gl_GlobalInvocationID.xy)"),
			"thread_position" => string.push_str("uint(gl_GlobalInvocationID.x)"),
			"thread_idx" => string.push_str("uint(gl_LocalInvocationIndex)"),
			"subgroup_lane_index" => string.push_str("gl_SubgroupInvocationID"),
			"threadgroup_position" => string.push_str("uint(gl_WorkGroupID.x)"),
			"subgroup_ballot_any" => {
				string.push_str("any(notEqual(");
				self.emit_node(string, &arguments[0]);
				string.push_str(", uvec4(0u)))");
			}
			"subgroup_ballot_and_not" => {
				string.push('(');
				self.emit_node(string, &arguments[0]);
				string.push_str(" & ~");
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			"workgroup_barrier" => string.push_str("barrier()"),
			// The task entry point emits the mesh grid from this count once `main` returns. See `mesh_shading.rs`.
			"set_task_mesh_output_count" => {
				let _ = write!(string, "{}=", mesh_shading::TASK_MESH_COUNT);
				self.emit_node(string, &arguments[0]);
			}
			"set_mesh_vertex_position" => {
				string.push_str("gl_MeshVerticesEXT[");
				self.emit_node(string, &arguments[0]);
				string.push_str("].gl_Position = ");
				self.emit_node(string, &arguments[1]);
			}
			"set_mesh_triangle" => {
				string.push_str("gl_PrimitiveTriangleIndicesEXT[");
				self.emit_node(string, &arguments[0]);
				string.push_str("] = ");
				self.emit_node(string, &arguments[1]);
			}
			"set_mesh_primitive_render_target_array_index" => {
				string.push_str("gl_MeshPrimitivesEXT[");
				self.emit_node(string, &arguments[0]);
				string.push_str("].gl_Layer = int(");
				self.emit_node(string, &arguments[1]);
				string.push(')');
			}
			// Every other intrinsic is a GLSL call, renamed where GLSL spells the operation differently.
			_ => {
				string.push_str(match name.as_str() {
					"atan2" => "atan",
					"is_nan" => "isnan",
					"is_infinite" => "isinf",
					"is_finite" => "_besl_is_finite",
					"is_normal" => "_besl_is_normal",
					"u32" => "uint",
					"f32" | "f16" | "u16" | "u8" | "vec2f" | "vec3f" | "vec4f" | "vec2f16" | "vec3f16" | "vec4f16" => {
						Self::type_from_besl(name)
					}
					"subgroup_ballot" => "subgroupBallot",
					"subgroup_ballot_find_lsb" => "subgroupBallotFindLSB",
					"subgroup_ballot_count" => "subgroupBallotBitCount",
					"subgroup_broadcast_u32" | "subgroup_broadcast_f32" => "subgroupBroadcast",
					"subgroup_shuffle_xor_f32" => "subgroupShuffleXor",
					"set_mesh_output_counts" => "SetMeshOutputsEXT",
					name => name,
				});
				string.push('(');
				self.emit_call_arguments(string, arguments);
				string.push(')');
			}
		}
	}
	fn emit_texture_parameter_sampler(&mut self, string: &mut String, name: &str) {
		self.emit_separator(string);
		let _ = write!(string, "uint {}_sampler", Self::identifier(name));
	}
	/// Replaces a call to a function with texture parameters, whose arguments each pass a texture and its sampler.
	fn emit_function_call(
		&mut self,
		string: &mut String,
		function: &besl::NodeReference,
		parameters: &[besl::NodeReference],
	) -> bool {
		let function = function.borrow();
		let besl::Nodes::Function { name, params, .. } = function.node() else {
			return false;
		};
		let takes_texture = |parameter: &besl::NodeReference| match parameter.borrow().node() {
			besl::Nodes::Parameter { r#type, .. } => r#type.borrow().get_name().is_some_and(is_texture_besl_type),
			_ => false,
		};
		if !params.iter().any(takes_texture) {
			return false;
		}
		Self::emit_type_name(string, name);
		string.push('(');
		for (index, argument) in parameters.iter().enumerate() {
			if index > 0 {
				self.emit_separator(string);
			}
			if params.get(index).is_some_and(takes_texture) {
				self.emit_texture_argument(string, argument);
			} else {
				self.emit_node(string, argument);
			}
		}
		string.push(')');
		true
	}
	fn emit_expression_member(&mut self, string: &mut String, name: &str, source: &besl::NodeReference) -> bool {
		// A descriptor lives in the heap, at its set's base index plus its offset in the set.
		if matches!(source.borrow().node(), besl::Nodes::Binding { .. }) {
			self.emit_binding_access(string, source, None);
			return true;
		}
		// A texture parameter holds heap indices, which a texture function reads through the heap aliases of its type.
		// A caller may pass an element any expression picks, so the indices may differ across a draw.
		if let besl::Nodes::Parameter { r#type, .. } = source.borrow().node()
			&& let Some(type_name) = r#type.borrow().get_name()
			&& is_texture_besl_type(type_name)
		{
			let parameter = Self::identifier(name);
			let _ = write!(
				string,
				"{}(_besl_{}[nonuniformEXT({parameter})],{SAMPLER_HEAP}[nonuniformEXT({parameter}_sampler)])",
				combined_sampler_type(type_name),
				parameter_texture_type(type_name),
			);
			return true;
		}
		let source = source.borrow();
		match source.node() {
			besl::Nodes::Input { name: input_name, .. } if name == input_name => match name {
				besl::VERTEX_INDEX_BUILTIN => string.push_str("uint(gl_VertexIndex)"),
				besl::INSTANCE_INDEX_BUILTIN => string.push_str("uint(gl_InstanceIndex)"),
				_ => return false,
			},
			besl::Nodes::Output {
				name: output_name,
				count: None,
				..
			} if self.stage.interpolates_outputs() && name == output_name && besl::is_position_output(name) => {
				string.push_str("gl_Position");
			}
			// Every task payload array is a member of the one payload variable.
			besl::Nodes::TaskPayload { .. } => {
				let _ = write!(string, "{}.", mesh_shading::TASK_PAYLOAD);
				Self::identifier(name).push_to(string);
			}
			_ => return false,
		}
		true
	}
	fn emit_accessor_expression(&mut self, string: &mut String, left: &besl::NodeReference, right: &besl::NodeReference) {
		// An element of a descriptor array is the heap element its index picks past the array's first one.
		if let Some(binding) = direct_binding(left)
			&& matches!(binding.borrow().node(), besl::Nodes::Binding { count: Some(_), .. })
		{
			self.emit_binding_access(string, &binding, Some(right));
			return;
		}
		self.emit_wrapped_expression(string, left);
		if !matches!(
			right.borrow().node(),
			besl::Nodes::Expression(besl::Expressions::Member { .. })
		) && left.borrow().node().is_indexable()
		{
			string.push('[');
			self.emit_node(string, right);
			string.push(']');
		} else {
			string.push('.');
			self.emit_node(string, right);
		}
	}

	// This function appends to the `string` parameter the string representation of the node.
	//
	// Example: Node::Literal { value: Literal::Float(3.14) } -> "3.14"
	// Example: Node::Struct { name: "Camera", fields: vec![Node::Field { name: "position", type: Type::Float }] } -> "struct Camera { float position; };"
	// Keep the exhaustive node-to-GLSL mapping together so adding a BESL node requires handling its backend contract here.
	#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
	fn emit_node(&mut self, string: &mut String, this_node: &besl::NodeReference) {
		let node = RefCell::borrow(this_node);
		let formatting = ShaderFormatting::new(self.minified);

		let break_char = formatting.break_str();
		let space_char = formatting.space_str();

		match node.node() {
			// A `use` and a signature only link names, and a layer's members emit where code reads them.
			besl::Nodes::Scope { .. } | besl::Nodes::Use { .. } | besl::Nodes::Layer { .. } | besl::Nodes::Signature { .. } => {}
			besl::Nodes::Function {
				name,
				statements,
				return_type,
				params,
				..
			} => {
				// Texture parameters read the heap aliases of their types, which are declared before the function.
				for parameter in params {
					if let besl::Nodes::Parameter { r#type, .. } = parameter.borrow().node()
						&& let Some(type_name) = r#type.borrow().get_name()
						&& is_texture_besl_type(type_name)
					{
						self.declare_parameter_texture(string, type_name);
					}
				}
				self.emit_function_node(string, this_node, name, statements, return_type, params)
			}
			// The ray tracing prelude declares `Hit` before the helpers that return it.
			besl::Nodes::Struct { name, .. } if name == besl::HIT_STRUCT => {}
			besl::Nodes::Struct {
				name, fields, template, ..
			} => self.emit_struct_node(string, name, fields, template),
			// The sets' base indices come first, and the program's own members start at `PUSH_CONSTANT_OFFSET`.
			besl::Nodes::PushConstant { members } => {
				let _ = write!(
					string,
					"layout(push_constant,{space_char}scalar){space_char}uniform PushConstant{space_char}{{{break_char}"
				);
				formatting.push_indentation(string, 1);
				let _ = write!(string, "uvec2 {SET_BASES}[{}]", besl::MAX_DESCRIPTOR_SETS);
				formatting.push_statement_end(string);
				let mut first = true;
				emit_statement_block(string, formatting, members, 1, |string, member| {
					if std::mem::take(&mut first) {
						let _ = write!(string, "layout(offset={PUSH_CONSTANT_OFFSET}){space_char}");
					}
					self.emit_node(string, member)
				});
				let _ = write!(string, "}}{space_char}push_constant;{break_char}");
			}
			besl::Nodes::Specialization { name, r#type, id } => self.emit_specialization_node(string, name, r#type, *id),
			besl::Nodes::Member { name, r#type, count } => {
				if let Some(type_name) = r#type.borrow().get_name() {
					// A member may be a user struct, which is declared under its escaped name.
					Self::type_identifier(type_name).push_to(string);
					string.push(' ');
				}
				Self::identifier(name).push_to(string);
				if let Some(count) = count {
					let _ = write!(string, "[{count}]");
				}
			}
			besl::Nodes::Parameter { name, r#type } => {
				self.emit_variable_declaration(string, name, r#type.borrow().get_name().unwrap())
			}
			besl::Nodes::Input {
				name,
				location,
				format,
				per_primitive,
			} => {
				if crate::shader::generator::is_vertex_builtin_input(name) {
					return;
				}
				let format = format.borrow();
				let besl_type = format.get_name().unwrap();
				let _ = write!(
					string,
					"layout(location={location}){space_char}{}{}in {} {};{break_char}",
					if self.stage.interpolates_inputs() && is_integer_besl_type(besl_type) {
						"flat "
					} else {
						""
					},
					// Vulkan links a per-primitive mesh output only to an input declared per-primitive too.
					if *per_primitive { "perprimitiveEXT " } else { "" },
					Self::type_from_besl(besl_type),
					Self::identifier(name)
				);
			}
			besl::Nodes::Output {
				name,
				location,
				format,
				count,
				per_vertex,
			} => {
				if count.is_none() && self.stage.interpolates_outputs() && besl::is_position_output(name) {
					return;
				}
				let format = format.borrow();
				let besl_type = format.get_name().unwrap();
				// Per-vertex mesh outputs interpolate like vertex-shader outputs, so integers stay flat.
				let interpolates = count.map_or(self.stage.interpolates_outputs(), |_| *per_vertex);
				let qualifier = if count.is_some() && !*per_vertex {
					"perprimitiveEXT "
				} else if interpolates && is_integer_besl_type(besl_type) {
					"flat "
				} else {
					""
				};
				let _ = write!(
					string,
					"layout(location={location}){space_char}{qualifier}out {} {}",
					Self::type_from_besl(besl_type),
					Self::identifier(name)
				);
				if let Some(count) = count {
					let _ = write!(string, "[{count}]");
				}
				let _ = write!(string, ";{break_char}");
			}
			besl::Nodes::Workgroup { name, format, count }
				if matches!(self.stage, Stages::Compute { .. } | Stages::Task { .. }) =>
			{
				let _ = write!(
					string,
					"shared {} {}",
					Self::type_identifier(format.borrow().get_name().unwrap()),
					Self::identifier(name)
				);
				if let Some(count) = count {
					let _ = write!(string, "[{count}]");
				}
				let _ = write!(string, ";{break_char}");
			}
			// The first payload array declares the payload struct, which holds all of them.
			besl::Nodes::TaskPayload { .. } => {
				if self.task_payloads.first() == Some(this_node) {
					self.emit_task_payload(string);
				}
			}
			// Stage validation admits workgroup storage only in compute and task stages.
			besl::Nodes::Workgroup { .. } => unreachable!("Expected workgroup storage only in compute and task stages"),
			besl::Nodes::Expression(expression) => self.emit_expression_node(string, expression),
			besl::Nodes::Conditional {
				condition,
				statements,
				else_branch,
			} => self.emit_conditional_node(string, condition, statements, else_branch.as_ref()),
			besl::Nodes::Match {
				scrutinee,
				r#type,
				arms,
				default,
			} => self.emit_match_node(string, scrutinee, r#type, arms, default),
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => self.emit_for_loop_node(string, initializer, condition, update, statements),
			// Each descriptor declares an unsized array that aliases the resource heap at 64-byte slots, which code
			// indexes from its set's base index. See `Generator::emit_binding_access`.
			besl::Nodes::Binding {
				name,
				read,
				write,
				r#type,
				placement,
				..
			} => {
				let Some(placement) = placement else {
					unreachable!("`generate` admits only placed bindings.");
				};
				self.declare_set(string, placement.set);
				// A sampled image is a texture without a sampler, which takes no format or access qualifier.
				let sampled_image = r#type.is_sampled_image(*read, *write);
				let binding_type = match r#type {
					besl::BindingTypes::Buffer { .. } | besl::BindingTypes::BufferArray { .. } => "buffer",
					besl::BindingTypes::Image { format, .. } => {
						let unsigned = matches!(format.as_str(), "r8ui" | "r16ui" | "r32ui");
						match (sampled_image, unsigned) {
							(true, true) => "uniform utexture2D",
							(true, false) => "uniform texture2D",
							(false, true) => "uniform uimage2D",
							(false, false) => "uniform image2D",
						}
					}
					// The texture alone lives in the resource heap; its sampler is combined with it where it is read.
					besl::BindingTypes::CombinedImageSampler { format, .. } => texture_declaration(format),
					besl::BindingTypes::AccelerationStructure => "uniform accelerationStructureEXT",
				};

				let _ = write!(string, "layout(descriptor_heap,descriptor_stride={RESOURCE_DESCRIPTOR_STRIDE}");
				match r#type {
					besl::BindingTypes::Buffer { .. } | besl::BindingTypes::BufferArray { .. } => string.push_str(",scalar"),
					besl::BindingTypes::Image { format } if format != "unknown" && !sampled_image => {
						string.push(',');
						string.push_str(format);
					}
					_ => {}
				}
				// Sampled images and acceleration structures take no access qualifier.
				let access = match r#type {
					besl::BindingTypes::CombinedImageSampler { .. } | besl::BindingTypes::AccelerationStructure => "",
					_ if sampled_image => "",
					_ if *read && !*write => "readonly ",
					_ if *write && !*read => "writeonly ",
					_ => "",
				};
				let _ = write!(string, ") {access}{binding_type} ");

				match r#type {
					besl::BindingTypes::Buffer { members } => {
						let _ = write!(string, "_{name}{{");
						for member in members {
							self.emit_node(string, member);
							self.emit_statement_end(string);
						}
						string.push('}');
					}
					// The block's one member has the binding's name, which `emit_binding_access` reads it through.
					besl::BindingTypes::BufferArray { element, fixed } => {
						let _ = write!(string, "_{name}{{");
						Self::emit_type_name(string, element.borrow().get_name().unwrap());
						string.push(' ');
						Self::identifier(name).push_to(string);
						// Runtime arrays leave the count to the bound buffer.
						match fixed {
							Some(fixed) => {
								let _ = write!(string, "[{}];", fixed.count);
							}
							None => string.push_str("[];"),
						}
						string.push('}');
					}
					besl::BindingTypes::Image { .. }
					| besl::BindingTypes::CombinedImageSampler { .. }
					| besl::BindingTypes::AccelerationStructure => {}
				}
				Self::identifier(name).push_to(string);
				string.push_str("[]");

				self.emit_statement_end(string);
			}
			besl::Nodes::Intrinsic { elements, .. } => {
				for element in elements {
					self.emit_node(string, element);
				}
			}
			besl::Nodes::Const { name, r#type, value } => {
				string.push_str("const ");
				Self::emit_type_name(string, r#type.borrow().get_name().unwrap());
				string.push(' ');
				Self::identifier(name).push_to(string);
				string.push_str(" = ");
				self.emit_node(string, value);
				let _ = write!(string, ";{break_char}");
			}
		}
	}
}
