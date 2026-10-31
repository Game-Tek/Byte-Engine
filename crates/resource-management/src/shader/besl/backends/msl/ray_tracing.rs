//! Lowers ray generation and closest-hit shaders, and the ray queries any kernel can run, to Metal.
//!
//! The lowering follows the contract between the Metal GHI backend and the shaders it links, which the ray tracing
//! design note (`docs/develop/rendering/ray-tracing.mdx`) describes. A ray generation shader becomes a compute kernel
//! that runs closest hits through the visible function table at [`SHADER_BINDING_TABLE_BINDING_INDEX`]. A closest hit
//! becomes a `[[visible]]` function that reaches its push constant and descriptor sets through the addresses the kernel
//! passes it, because a visible function has no argument table of its own.

use super::*;

/// Names the kernel parameter that holds the point of the launch grid an invocation runs for.
pub(crate) const LAUNCH_INDEX_ARGUMENT: &str = "_besl_launch_index";

/// Names the kernel parameter that holds the launch size of the trace.
pub(crate) const LAUNCH_SIZE_ARGUMENT: &str = "_besl_launch_size";

/// Names the ray generation kernel's shader binding table, which holds the closest hit of each record.
pub(crate) const HITS_ARGUMENT: &str = "_besl_hits";

/// Names the addresses that a ray generation kernel passes to the closest hits it runs.
pub(crate) const CONTEXT_ARGUMENT: &str = "_besl_context";

/// Names the hit that a closest hit shades.
pub(crate) const HIT_ARGUMENT: &str = "_besl_hit";

/// The struct that holds the addresses of the push constant and of each descriptor set that a ray generation kernel
/// passes to closest hits.
pub(crate) const CONTEXT_STRUCT: &str = "_besl_ray_context";

/// The function type of every closest hit: it updates the payload from the hit and the kernel's addresses.
pub(crate) const CLOSEST_HIT_TYPE: &str = "_besl_closest_hit";

/// Names the function that runs a closest hit's `main`, which the visible entry point calls.
const CLOSEST_HIT_MAIN: &str = "_besl_closest_hit_main";

/// Names the visible entry point's payload parameter.
const PAYLOAD_ARGUMENT: &str = "_besl_payload";

impl Generator {
	/// Reports whether the shader needs Metal's ray tracing library: it is a ray tracing stage, runs a ray query, or
	/// declares a `Hit` or an acceleration structure, in its own code or in a descriptor set it takes.
	pub(crate) fn uses_ray_tracing(
		&self,
		stage: Stages,
		order: &[besl::NodeReference],
		requirements: &IntrinsicRequirements,
	) -> bool {
		let declares_ray_tracing = super::super::declares_ray_tracing;
		stage.is_ray_tracing()
			|| requirements.uses_intersect
			|| requirements.uses_trace_occlusion
			|| order.iter().any(declares_ray_tracing)
			|| self.argument_sets.iter().flatten().any(declares_ray_tracing)
	}

	/// Emits what a shader that uses ray tracing declares before its own code: the `Hit` struct, the addresses a ray
	/// tracing stage passes to closest hits, and the helpers of the ray queries the shader runs.
	///
	/// The shader's own emission skips the `Hit` node, so the struct is declared once, before the helpers return it.
	pub(crate) fn emit_ray_tracing_prelude(&self, string: &mut String, stage: Stages, requirements: &IntrinsicRequirements) {
		// Vectors are packed, as in every struct a buffer holds, so a hit has one layout in buffers and in calls.
		self.emit_named_struct_start(string, besl::HIT_STRUCT);
		for (name, r#type) in besl::HIT_MEMBERS {
			self.emit_indentation(string, 1);
			Self::emit_buffer_member_type(string, r#type);
			string.push(' ');
			Self::identifier(name).push_to(string);
			self.emit_statement_end(string);
		}
		self.emit_struct_declaration_end(string);

		if stage.is_ray_tracing() {
			// A closest hit finds set `i` at position `i`. Its sets are the kernel's first sets, so the struct it declares
			// is a prefix of the kernel's, and both read the same addresses.
			let _ = writeln!(
				string,
				"struct {CONTEXT_STRUCT} {{ constant void* push_constant; constant void* sets[{}]; }};",
				self.argument_sets.len().max(1)
			);
		}

		let hit = besl::HIT_STRUCT;
		if requirements.uses_intersect {
			// Only an intersector tagged `intersection_function_buffer` reports `function_id`, which is the hit's record.
			// Every triangle is opaque, so no geometry calls an intersection function, and the default, empty function
			// buffer is enough.
			let _ = write!(
				string,
				"inline {hit} _besl_intersect({ACCELERATION_STRUCTURE_TYPE} scene, uint mask, float3 origin, float3 direction, float t_min, float t_max) {{\n\
				 \traytracing::intersector<raytracing::instancing, raytracing::triangle_data, raytracing::intersection_function_buffer> trace;\n\
				 \ttrace.assume_geometry_type(raytracing::geometry_type::triangle);\n\
				 \ttrace.force_opacity(raytracing::forced_opacity::opaque);\n\
				 \tconst auto result = trace.intersect(raytracing::ray(origin, direction, t_min, t_max), scene, mask, raytracing::intersection_function_buffer_arguments());\n\
				 \tif (result.type == raytracing::intersection_type::none) {{ return {hit}{{0u, -1.0f, 0u, 0u, 0u, float2(0.0f), origin, direction}}; }}\n\
				 \treturn {hit}{{result.function_id, result.distance, result.user_instance_id, result.geometry_id, result.primitive_id, result.triangle_barycentric_coord, origin, direction}};\n\
				 }}\n"
			);
		}
		if requirements.uses_trace_occlusion {
			// Any triangle answers the query, so the intersector stops at the first one and reports no triangle data.
			let _ = write!(
				string,
				"inline bool _besl_trace_occlusion({ACCELERATION_STRUCTURE_TYPE} scene, uint mask, float3 origin, float3 direction, float t_min, float t_max) {{\n\
				 \traytracing::intersector<raytracing::instancing> trace;\n\
				 \ttrace.assume_geometry_type(raytracing::geometry_type::triangle);\n\
				 \ttrace.force_opacity(raytracing::forced_opacity::opaque);\n\
				 \ttrace.accept_any_intersection(true);\n\
				 \treturn trace.intersect(raytracing::ray(origin, direction, t_min, t_max), scene, mask).type != raytracing::intersection_type::none;\n\
				 }}\n"
			);
		}
	}

	/// Generates a ray generation shader: a compute kernel over the launch grid, which runs closest hits with `invoke`.
	///
	/// The kernel takes every descriptor set and the push constant its program declares, read or not, because it
	/// passes their addresses to the closest hits it runs, and a closest hit can read what the kernel doesn't.
	pub(crate) fn generate_ray_generation_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main: &besl::NodeReference,
	) {
		let nodes = Self::classify_nodes(order);
		self.emit_storage_declarations(string, &nodes.declarations, None);
		if let Some(push_constant) = nodes.push_constant {
			self.emit_push_constant_struct(string, push_constant);
		}
		self.emit_buffer_binding_structs(string, &nodes.bindings);
		self.emit_argument_buffer_structs(string);

		let payload = invoke_payload(order);
		if let Some(payload) = &payload {
			self.emit_invoke_helper(string, payload);
		}

		let context = RayTracingStageContext {
			closest_hit: false,
			has_push_constant: nodes.push_constant.is_some(),
			invokes: payload.is_some(),
		};
		let previous_ray_tracing_stage_context = self.ray_tracing_stage_context.replace(context);
		self.emit_functions(string, &nodes.functions);
		self.emit_ray_generation_entry_point(string, main, context);
		self.ray_tracing_stage_context = previous_ray_tracing_stage_context;
	}

	/// Declares the closest-hit function type for `payload`, and `_besl_invoke`, which runs the closest hit of a hit's
	/// record and returns the payload it leaves, or returns the payload of a miss unchanged.
	fn emit_invoke_helper(&self, string: &mut String, payload: &besl::NodeReference) {
		let payload = payload.borrow();
		let payload = Self::type_identifier(payload.get_name().unwrap_or_default());
		let hit = besl::HIT_STRUCT;
		let distance = Self::identifier("distance");
		// `>=` is false for a negative distance and for one that isn't a number, which are both misses.
		let _ = write!(
			string,
			"using {CLOSEST_HIT_TYPE} = void(thread {payload}&, thread const {hit}&, thread const {CONTEXT_STRUCT}&);\n\
			 inline {payload} _besl_invoke(visible_function_table<{CLOSEST_HIT_TYPE}> hits, thread const {CONTEXT_STRUCT}& context, {hit} hit, {payload} payload) {{\n\
			 \tif (hit.{distance} >= 0.0f) {{ hits[hit.record](payload, hit, context); }}\n\
			 \treturn payload;\n\
			 }}\n"
		);
	}

	/// Writes the kernel of a ray generation shader. When the shader calls `invoke`, the kernel first collects the
	/// addresses of its push constant and descriptor sets, which every closest hit it runs reads.
	fn emit_ray_generation_entry_point(
		&mut self,
		string: &mut String,
		main: &besl::NodeReference,
		context: RayTracingStageContext,
	) {
		let node = main.borrow();
		let besl::Nodes::Function { statements, .. } = node.node() else {
			return;
		};

		let _ = write!(
			string,
			"kernel void {MSL_ENTRY_POINT}(uint3 {LAUNCH_INDEX_ARGUMENT} [[thread_position_in_grid]]"
		);
		self.emit_separator(string);
		let _ = write!(string, "uint3 {LAUNCH_SIZE_ARGUMENT} [[threads_per_grid]]");
		if context.invokes {
			self.emit_separator(string);
			let _ = write!(
				string,
				"visible_function_table<{CLOSEST_HIT_TYPE}> {HITS_ARGUMENT} [[buffer({SHADER_BINDING_TABLE_BINDING_INDEX})]]"
			);
		}
		if context.has_push_constant {
			self.emit_separator(string);
			self.emit_push_constant_parameter(string);
		}
		if self.has_argument_sets() {
			self.emit_separator(string);
			self.emit_argument_buffer_parameter(string);
		}
		ShaderFormatting::new(self.minified).push_block_start(string);

		if context.invokes {
			self.emit_indentation(string, 1);
			let _ = write!(string, "const {CONTEXT_STRUCT} {CONTEXT_ARGUMENT}={{");
			string.push_str(if context.has_push_constant {
				"&push_constant"
			} else {
				"nullptr"
			});
			self.emit_separator(string);
			string.push('{');
			for set in 0..self.argument_sets.len() {
				if set > 0 {
					self.emit_separator(string);
				}
				let _ = write!(string, "&{}", SetArgument(set));
			}
			if !self.has_argument_sets() {
				string.push_str("nullptr");
			}
			string.push_str("}}");
			self.emit_statement_end(string);
		}
		self.emit_statement_block(string, statements, 1);
		self.emit_block_end(string);
	}

	/// Generates a closest-hit shader: a `[[visible]]` function named [`MSL_ENTRY_POINT`] that a ray generation kernel
	/// calls through its shader binding table.
	///
	/// The function finds its push constant and descriptor sets at the addresses the kernel passes, then runs `main`,
	/// which takes and returns the payload, and leaves the result in the kernel's payload. Fails when `main` doesn't
	/// take the payload as its only parameter.
	pub(crate) fn generate_closest_hit_shader(
		&mut self,
		string: &mut String,
		order: &[besl::NodeReference],
		main: &besl::NodeReference,
	) -> Result<(), ()> {
		let nodes = Self::classify_nodes(order);
		self.emit_storage_declarations(string, &nodes.declarations, None);
		if let Some(push_constant) = nodes.push_constant {
			self.emit_push_constant_struct(string, push_constant);
		}
		self.emit_buffer_binding_structs(string, &nodes.bindings);
		self.emit_argument_buffer_structs(string);

		let context = RayTracingStageContext {
			closest_hit: true,
			has_push_constant: nodes.push_constant.is_some(),
			invokes: false,
		};
		let previous_ray_tracing_stage_context = self.ray_tracing_stage_context.replace(context);
		self.emit_functions(string, &nodes.functions);
		let result = self.emit_closest_hit_entry_point(string, main, context);
		self.ray_tracing_stage_context = previous_ray_tracing_stage_context;
		result
	}

	/// Writes a closest hit's `main` as a function that returns the payload, then the visible entry point that unpacks
	/// the kernel's addresses and calls it.
	fn emit_closest_hit_entry_point(
		&mut self,
		string: &mut String,
		main: &besl::NodeReference,
		context: RayTracingStageContext,
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
		// Entry normalization leaves the payload as the only parameter of a `main` that returns it.
		if params.len() != 1 {
			return Err(());
		}
		let payload = return_type.borrow();
		let payload = Self::type_identifier(payload.get_name().ok_or(())?);

		// `main` stays a function of its own, so its `return` statements keep returning the payload.
		let _ = write!(string, "{payload} {CLOSEST_HIT_MAIN}(");
		self.emit_function_parameters(string, params);
		self.emit_hidden_context(string, main, true, true);
		ShaderFormatting::new(self.minified).push_block_start(string);
		self.emit_statement_block(string, statements, 1);
		self.emit_block_end(string);

		let _ = write!(
			string,
			"[[visible]] void {MSL_ENTRY_POINT}(thread {payload}& {PAYLOAD_ARGUMENT}"
		);
		self.emit_separator(string);
		let _ = write!(string, "thread const {}& {HIT_ARGUMENT}", besl::HIT_STRUCT);
		self.emit_separator(string);
		let _ = write!(string, "thread const {CONTEXT_STRUCT}& {CONTEXT_ARGUMENT}");
		ShaderFormatting::new(self.minified).push_block_start(string);
		if context.has_push_constant {
			self.emit_indentation(string, 1);
			let _ = write!(
				string,
				"constant PushConstant& push_constant=*static_cast<constant PushConstant*>({CONTEXT_ARGUMENT}.push_constant)"
			);
			self.emit_statement_end(string);
		}
		for set in 0..self.argument_sets.len() {
			self.emit_indentation(string, 1);
			let _ = write!(
				string,
				"constant _set{set}& {}=*static_cast<constant _set{set}*>({CONTEXT_ARGUMENT}.sets[{set}])",
				SetArgument(set)
			);
			self.emit_statement_end(string);
		}
		self.emit_indentation(string, 1);
		let _ = write!(string, "{PAYLOAD_ARGUMENT}={CLOSEST_HIT_MAIN}({PAYLOAD_ARGUMENT}");
		self.emit_hidden_context(string, main, true, false);
		string.push(')');
		self.emit_statement_end(string);
		self.emit_block_end(string);
		Ok(())
	}

	/// Writes the value of a closest hit's stage value `name`, such as `hit_distance`: a member of the hit it shades.
	pub(crate) fn emit_hit_value(&self, string: &mut String, name: &str) {
		let member = super::super::hit_member(name);
		// A hit's vectors are packed, and a stage value is a native vector, as its `StageInput` field is.
		let unpack = match name {
			"hit_barycentrics" => Some("float2"),
			"ray_origin" | "ray_direction" => Some("float3"),
			_ => None,
		};
		if let Some(vector) = unpack {
			let _ = write!(string, "{vector}({HIT_ARGUMENT}.{})", Self::identifier(member));
		} else {
			let _ = write!(string, "{HIT_ARGUMENT}.{}", Self::identifier(member));
		}
	}
}
