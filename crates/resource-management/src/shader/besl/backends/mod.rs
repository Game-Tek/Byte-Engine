/// Asserts that a generated shader contains `needle`, and prints the shader when it does not.
#[cfg(test)]
macro_rules! assert_string_contains {
	($haystack:expr, $needle:expr) => {
		assert!(
			$haystack.contains($needle),
			"Expected string to contain '{}', but it did not. String: '{}'",
			$needle,
			$haystack
		);
	};
}

/// Asserts that a generated shader does not contain `needle`, and prints the shader when it does.
#[cfg(test)]
macro_rules! assert_string_does_not_contain {
	($haystack:expr, $needle:expr) => {
		assert!(
			!$haystack.contains($needle),
			"Expected string not to contain '{}', but it did. String: '{}'",
			$needle,
			$haystack
		);
	};
}

pub mod glsl;
pub mod hlsl;
pub mod msl;
pub mod platform;
pub mod spirv;
mod stage_rules;

pub(crate) use stage_rules::validate_stage;

/// The layout layers that the ray tracing fixtures list, in the order every shader of their pipeline lists them:
/// the payload, the scene and the results the ray generation shader writes, the albedos that only closest hits
/// read, and the push constant.
#[cfg(test)]
const RAY_TRACING_LAYERS: [(&str, &str); 4] = [
	("surface", "SurfaceHit: struct { albedo: vec3f, distance: f32, }"),
	(
		"scene",
		"scene: descriptor<{ type: AccelerationStructure, binding: 0, access: read }>; surfaces: descriptor<{ type: SurfaceHit[], binding: 1, access: write }>;",
	),
	(
		"materials",
		"albedos: descriptor<{ type: vec4f[], binding: 2, access: read }>;",
	),
	("constants", "push_constant: push_constant { scale: f32, }"),
];

/// Stacks the ray tracing fixture layers under `source` and links the program.
#[cfg(test)]
fn link_ray_tracing_fixture(source: &str) -> besl::NodeReference {
	let module = besl::parse(source).expect("Expected the ray tracing fixture to parse");
	besl::lex(besl::stack_layers(module, RAY_TRACING_LAYERS).expect("Expected the layers to stack"))
		.expect("Expected the ray tracing fixture to link")
}

/// Shades one ray for each launch point, through a helper that calls `invoke`, and marks occluded launch points.
#[cfg(test)]
const RAY_GENERATION_FIXTURE: &str = r#"
	shade: fn (hit: Hit) -> SurfaceHit {
		return invoke(hit, SurfaceHit(vec3f(0.0, 0.0, 0.0), -1.0));
	}

	main: fn (input: StageInput) -> void {
		let origin: vec3f = vec3f(f32(input.launch_index.x) * push_constant.scale, 0.0, 0.0);
		let hit: Hit = intersect(scene, 255, origin, vec3f(0.0, 0.0, 2.0), 0.0, 100.0);
		let surface: SurfaceHit = shade(hit);
		if (trace_occlusion(scene, 255, origin, vec3f(0.0, 0.0, 1.0), 0.0, 100.0)) {
			surface.distance = surface.distance + f32(input.launch_size.x);
		}
		surfaces[input.launch_index.x] = surface;
	}
"#;

/// Reports a hit's albedo from a set the ray generation shader doesn't read, through a helper that reads the hit.
#[cfg(test)]
const CLOSEST_HIT_FIXTURE: &str = r#"
	albedo_of: fn (geometry: u32) -> vec3f {
		let albedo: vec4f = albedos[geometry];
		return vec3f(albedo.x, albedo.y, albedo.z) * hit_distance();
	}

	main: fn (input: StageInput, payload: SurfaceHit) -> SurfaceHit {
		let surface: SurfaceHit = payload;
		let weights: vec2f = input.hit_barycentrics;
		surface.albedo = albedo_of(input.hit_geometry_index) + vec3f(weights.x, weights.y, f32(input.hit_primitive_index));
		surface.distance = input.hit_distance * push_constant.scale + input.ray_direction.z;
		return surface;
	}
"#;

/// Compacts the visible meshlets of one task workgroup into its payload through workgroup atomics, and sets the mesh
/// grid from a helper on one lane.
#[cfg(test)]
const TASK_PAYLOAD_FIXTURE: &str = r#"
	Meshlets: struct {
		values: u32[32],
	}
	meshlets: descriptor<{ type: Meshlets, binding: 8, access: read }>;
	visible_meshlets: task_payload<u32, 32>;
	visible_count: workgroup<atomicu32>;
	push_constant: push_constant {
		base_meshlet: u32,
	}

	dispatch_visible_meshlets: fn () -> void {
		let position: u32 = thread_position();
		let lane: u32 = thread_idx();
		if (lane == 0) {
			atomic_store(visible_count, 0);
		}
		workgroup_barrier();
		if (position < 32) {
			let payload_index: u32 = atomic_add(visible_count, 1);
			visible_meshlets[payload_index] = meshlets.values[push_constant.base_meshlet + position];
		}
		workgroup_barrier();
		if (lane == 0) {
			set_task_mesh_output_count(atomic_load(visible_count));
		}
	}

	main: fn () -> void {
		dispatch_visible_meshlets();
	}
"#;

/// Reads the meshlet its workgroup draws from the task payload, sets its counts, and writes one triangle with
/// per-primitive outputs.
#[cfg(test)]
const MESH_PAYLOAD_FIXTURE: &str = r#"
	visible_meshlets: task_payload<u32, 32>;
	out_instance_index: output<u32, 0, 126>;
	out_primitive_index: output<u32, 1, 126>;

	main: fn () -> void {
		let lane: u32 = thread_idx();
		let meshlet_index: u32 = visible_meshlets[threadgroup_position()];
		set_mesh_output_counts(3, 1);
		if (lane < 3) {
			set_mesh_vertex_position(lane, vec4f(f32(lane), 0.0, 0.0, 1.0));
		}
		if (lane < 1) {
			set_mesh_triangle(0, vec3u(0, 1, 2));
			out_primitive_index[0] = meshlet_index;
			set_mesh_primitive_render_target_array_index(0, 2);
			out_instance_index[0] = meshlet_index;
		}
	}
"#;

/// Runs baked ray tracing shaders on the GPU and in the VM. It needs GHI, which the `gpu-processing` feature brings, and
/// a backend whose generator lowers ray tracing.
#[cfg(all(test, feature = "gpu-processing", any(target_os = "macos", target_os = "linux")))]
mod gpu_ray_tracing;

/// Draws with baked task, mesh, and fragment shaders on the GPU. It needs GHI, which the `gpu-processing` feature
/// brings, and a backend whose generator lowers task and mesh stages.
#[cfg(all(test, feature = "gpu-processing", any(target_os = "macos", target_os = "linux")))]
mod gpu_mesh_shading;

/// Bakes programs and creates what runs them for the GPU tests.
#[cfg(all(test, feature = "gpu-processing", any(target_os = "macos", target_os = "linux")))]
mod gpu_harness;

#[cfg(test)]
const RUNTIME_ARRAY_FRAGMENT: &str = r#"
	Instance: struct { position: vec3f, sprite_id: u32 }
	sprites: descriptor<{ type: Texture2DArray, binding: 0, access: read }>;
	instances: descriptor<{ type: Instance[], binding: 1, access: read }>;
	main: fn (input: StageInput, pipeline_input: interface { instance_index: u32, uv: vec2f }) -> output { color: vec4f } {
		let instance: Instance = instances[pipeline_input.instance_index];
		let color: vec4f = sample(sprites[instance.sprite_id], pipeline_input.uv);
		return { color };
	}
"#;

/// Reads packed vector and narrow scalar runtime arrays, then writes a scalar runtime array.
#[cfg(test)]
const SCALAR_RUNTIME_ARRAY_COMPUTE: &str = r#"
	positions: descriptor<{ type: vec3f[], binding: 0, access: read }>;
	indices: descriptor<{ type: u16[], binding: 1, access: read }>;
	corners: descriptor<{ type: u8[], binding: 2, access: read }>;
	results: descriptor<{ type: u32[], binding: 3, access: write }>;
	main: fn (input: StageInput) -> void {
		let item: u32 = input.thread_id.x;
		let index: u32 = u32(indices[item]) + u32(corners[item]);
		let position: vec3f = positions[index];
		results[item] = u32(position.x);
	}
"#;

#[cfg(test)]
const STRUCTURAL_POSITION_VERTEX: &str = r#"
	main: fn (input: StageInput) -> interface { position: vec4f, uv: vec2f } {
		let position: vec4f = vec4f(f32(input.vertex_index), 0.0, 0.0, 1.0);
		let uv: vec2f = vec2f(0.0, 0.0);
		return { position, uv };
	}
"#;

#[cfg(test)]
const DESCRIPTOR_ARRAY_FRAGMENT: &str = r#"
	Item: struct { slot: u32 }
	items: descriptor<{ type: Item[], binding: 0, access: read }>;
	textures: descriptor<{ type: Texture2D, binding: 1, access: read, count: 4 }>;
	shade: fn (index: u32, uv: vec2f) -> vec4f {
		let size: vec2u = texture_size(textures[items[index].slot]);
		let lod: vec4f = texture_lod(textures[index + 1], uv);
		let nested: vec4f = sample(textures[items[index].slot], uv);
		let texel: vec4f = fetch(textures[index], vec2u(0, 0));
		let quad: vec4f = gather(textures[index], uv);
		return (lod + nested + texel + quad) * f32(size.x);
	}
	main: fn (input: StageInput, pipeline_input: interface { index: u32, uv: vec2f }) -> output { color: vec4f } {
		let color: vec4f = shade(pipeline_input.index, pipeline_input.uv);
		return { color };
	}
"#;

/// Uses every subgroup ballot, broadcast, and shuffle operation in a compute shader.
#[cfg(test)]
const SUBGROUP_COMPUTE: &str = r#"
	main: fn () -> void {
		let mask: vec4u = subgroup_ballot(thread_idx() < 4);
		let leader: u32 = subgroup_ballot_find_lsb(mask);
		let value: u32 = subgroup_broadcast_u32(thread_idx(), leader);
		let neighbor: f32 = subgroup_shuffle_xor_f32(f32(value), 1);
		let remaining: vec4u = subgroup_ballot_and_not(mask, subgroup_ballot(value == 0));
		if (subgroup_ballot_any(remaining)) {
			let count: u32 = subgroup_ballot_count(remaining);
			count;
			neighbor;
		}
	}
"#;

/// Reads the vertex invocation indices inside a helper function.
#[cfg(test)]
const VERTEX_BUILTIN_HELPER: &str = r#"
	invocation_sum: fn () -> u32 {
		return vertex_index + instance_index;
	}
	out_value: output<u32, 0>;
	main: fn () -> void {
		out_value = invocation_sum();
	}
"#;

/// Returns, passes, and indexes short scalar arrays, which backends lower to vectors.
#[cfg(test)]
const SHORT_SCALAR_ARRAYS: &str = r#"
	scalar_f32: fn () -> f32[3] {
		return f32[3](0.5, 0.25, 0.125);
	}
	scalar_u16: fn () -> u16[3] {
		return u16[3](1, 2, 3);
	}
	scalar_u32: fn () -> u32[3] {
		return u32[3](4, 5, 6);
	}
	mirror_indices: fn (indices: u32[3]) -> u32[3] {
		return indices;
	}
	main: fn () -> void {
		let floats: f32[3] = scalar_f32();
		let shorts: u16[3] = scalar_u16();
		let indices: u32[3] = mirror_indices(scalar_u32());
		let sum: f32 = floats[1] + f32(u32(shorts[1])) + f32(indices[1]);
		sum;
	}
"#;

/// Chains `else if` and `else` branches.
#[cfg(test)]
const ELSE_CHAIN: &str = r#"
	main: fn () -> void {
		let n: u32 = 0;
		if (n < 1) {
			n = 2;
		} else if (n < 4) {
			n = 3;
		} else {
			n = 4;
		}
	}
"#;

/// Finds the lowest set bit of an unsigned value.
#[cfg(test)]
const FIND_LSB: &str = r#"
	main: fn () -> void {
		let bits: u32 = 40;
		let lowest: u32 = find_lsb(bits);
		lowest;
	}
"#;

/// Reads a module-level constant array.
#[cfg(test)]
const CONST_ARRAY: &str = r#"
	WEIGHTS: const f32[3] = f32[3](0.5, 0.25, 0.125);

	main: fn () -> void {
		let value: f32 = WEIGHTS[1];
		value;
	}
"#;

/// Reads a member and an element of parenthesized values, whose parentheses must survive lowering.
#[cfg(test)]
const GROUPED_ACCESSOR_BASES: &str = r#"
	main: fn () -> void {
		let v: vec2f = vec2f(1.0, 2.0);
		let w: vec2f = vec2f(3.0, 4.0);
		let sum_y: f32 = (v + w).y;
		let values: u32[2] = u32[2](1, 2);
		let i: u32 = 1;
		let picked: u32 = (values)[i];
		sum_y;
		picked;
	}
"#;

/// Uses an `if` and a `match` whose branches run statements, so the lexer hoists each in front of its statement, and a
/// left operand that runs before the `match` is captured first.
#[cfg(test)]
const BRANCH_VALUES: &str = r#"
	main: fn () -> void {
		let n: u32 = 3;
		let scale: f32 = 2.0;
		let weight: f32 = scale * match n {
			0 => 0.5,
			1 | 2 => {
				let half_scale: f32 = scale * 0.5;
				half_scale + 1.0
			}
			_ => 1.0,
		};
		let shade: f32 = if (n > 1) {
			let lit: f32 = weight * weight;
			lit
		} else {
			0.0
		};
		shade;
	}
"#;

/// Matches on a loop counter, with a `break` in one arm, and on a narrow integer.
#[cfg(test)]
const MATCH_IN_LOOP: &str = r#"
	main: fn () -> void {
		let n: u32 = 0;
		let small: u16 = u16(n);
		for (let i: u32 = 0; i < 4; i = i + 1) {
			match i {
				0 => n = 1,
				1 | 2 => break,
				_ => {}
			}
		}
		match small {
			65535 => n = 2,
			_ => n = 3,
		}
	}
"#;

/// Identifies the two resource operations that use BESL accessor syntax.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResourceAccessorKind {
	DescriptorArray,
	Texture2DArrayLayer,
}

/// Classifies one resource accessor without relying on its surface syntax alone.
fn resource_accessor(node: &besl::NodeReference) -> Option<(ResourceAccessorKind, besl::NodeReference, besl::NodeReference)> {
	let node = node.borrow();
	let besl::Nodes::Expression(besl::Expressions::Accessor { left, right }) = node.node() else {
		return None;
	};
	let kind = resource_reference_kind(left)?;
	Some((kind, left.clone(), right.clone()))
}

/// Recovers resource metadata through the linked member expression used for an identifier.
fn resource_reference_kind(node: &besl::NodeReference) -> Option<ResourceAccessorKind> {
	match node.borrow().node() {
		besl::Nodes::Binding {
			r#type: besl::BindingTypes::CombinedImageSampler { format },
			count,
			..
		} => {
			if count.is_some() {
				Some(ResourceAccessorKind::DescriptorArray)
			} else if format == "ArrayTexture2D" {
				Some(ResourceAccessorKind::Texture2DArrayLayer)
			} else {
				None
			}
		}
		besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => resource_reference_kind(source),
		_ => None,
	}
}

/// Returns the element type when `node` refers to a runtime storage-buffer array.
fn runtime_buffer_element(node: &besl::NodeReference) -> Option<besl::NodeReference> {
	match node.borrow().node() {
		besl::Nodes::Binding {
			r#type: besl::BindingTypes::BufferArray { element, .. },
			..
		} => Some(element.clone()),
		besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => runtime_buffer_element(source),
		_ => None,
	}
}

/// Returns whether a linked expression is the scalar value two, optionally wrapped in a scalar cast.
fn is_two(node: &besl::NodeReference) -> bool {
	match node.borrow().node() {
		besl::Nodes::Expression(besl::Expressions::Literal { value }) => value.parse::<f64>() == Ok(2.0),
		besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) if arguments.len() == 1 && matches!(intrinsic.borrow().get_name(), Some("f16" | "f32")) => is_two(&arguments[0]),
		_ => false,
	}
}

/// Reports whether `predicate` holds for `node` or for any code nested in it, stopping at the first match.
///
/// Backends use it to scan shader code for intrinsics or constructs they must declare, enable, or reject. The walk
/// follows [`besl::Nodes::children`], visits an intrinsic call's arguments but not its expansion, and visits the
/// declaration a member expression reads. Set `follow_calls` to also search the bodies of called functions, for
/// properties a caller inherits from its callees, such as a hidden stage parameter.
fn any_code_node<F: FnMut(&besl::NodeReference) -> bool>(
	node: &besl::NodeReference,
	follow_calls: bool,
	predicate: &mut F,
) -> bool {
	if predicate(node) {
		return true;
	}
	let mut visit = |child: &besl::NodeReference| any_code_node(child, follow_calls, predicate);
	match node.borrow().node() {
		besl::Nodes::Expression(besl::Expressions::IntrinsicCall { arguments, .. }) => arguments.iter().any(visit),
		besl::Nodes::Expression(besl::Expressions::Member { source, .. }) => visit(source),
		besl::Nodes::Expression(besl::Expressions::FunctionCall { function, parameters }) => {
			(follow_calls && visit(&function.get())) || parameters.iter().any(visit)
		}
		// A constant's value is compile-time data, not code that runs where the constant is read.
		besl::Nodes::Const { .. } => false,
		other => other.children().any(visit),
	}
}

/// Reports whether `node`, or code nested in it, calls the intrinsic named `intrinsic_name`.
///
/// Backends use it to enable extensions or declare helpers only for shaders that need them. It does not search the
/// bodies of called functions, so pass every emitted function, as [`crate::shader::generator::ordered_shader_nodes`]
/// returns them, to cover a whole shader.
fn uses_intrinsic(node: &besl::NodeReference, intrinsic_name: &str) -> bool {
	any_code_node(node, false, &mut |node| is_intrinsic_call(node, intrinsic_name))
}

/// Reports whether `node` is a call to the intrinsic named `intrinsic_name`.
fn is_intrinsic_call(node: &besl::NodeReference, intrinsic_name: &str) -> bool {
	matches!(
		node.borrow().node(),
		besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. })
			if intrinsic.borrow().get_name() == Some(intrinsic_name)
	)
}

/// The BESL subgroup operations, which every backend supports only in compute shaders.
const SUBGROUP_INTRINSICS: [&str; 9] = [
	"subgroup_lane_index",
	"subgroup_ballot",
	"subgroup_ballot_any",
	"subgroup_ballot_find_lsb",
	"subgroup_ballot_count",
	"subgroup_ballot_and_not",
	"subgroup_broadcast_u32",
	"subgroup_broadcast_f32",
	"subgroup_shuffle_xor_f32",
];

/// The BESL calls that read a ray tracing stage's own values: the launch point of a ray generation shader, the hit a
/// closest-hit shader shades, and `invoke`, which a ray generation shader calls through its shader binding table.
const RAY_TRACING_STAGE_INTRINSICS: [&str; 10] = [
	"launch_index",
	"launch_size",
	"hit_distance",
	"hit_custom_index",
	"hit_geometry_index",
	"hit_primitive_index",
	"hit_barycentrics",
	"ray_origin",
	"ray_direction",
	"invoke",
];

/// Reports whether `node` declares ray tracing, the `Hit` struct or an acceleration structure binding, which a shader
/// that declares either needs its backend's ray tracing support for.
pub(crate) fn declares_ray_tracing(node: &besl::NodeReference) -> bool {
	match node.borrow().node() {
		besl::Nodes::Struct { name, .. } => name == besl::HIT_STRUCT,
		besl::Nodes::Binding {
			r#type: besl::BindingTypes::AccelerationStructure,
			..
		} => true,
		_ => false,
	}
}

/// Returns the `Hit` member that the closest-hit stage value `name`, such as `hit_distance`, reads.
pub(crate) fn hit_member(name: &str) -> &'static str {
	match name {
		"hit_distance" => "distance",
		"hit_custom_index" => "custom_index",
		"hit_geometry_index" => "geometry_index",
		"hit_primitive_index" => "primitive_index",
		"hit_barycentrics" => "barycentrics",
		"ray_origin" => "origin",
		"ray_direction" => "direction",
		_ => unreachable!("`{name}` isn't a hit value. Add it to `hit_member` with the `Hit` member it reads."),
	}
}

/// Returns the payload type that the `invoke` calls in `order` pass, or `None` when no code calls `invoke`.
///
/// The lexer makes every `invoke` call of a program pass one payload type, so the first call names it.
pub(crate) fn invoke_payload(order: &[besl::NodeReference]) -> Option<besl::NodeReference> {
	let mut payload = None;
	for node in order {
		any_code_node(node, false, &mut |node| {
			if let besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) = node.borrow().node()
				&& let besl::Nodes::Intrinsic { name, r#return, .. } = intrinsic.borrow().node()
				&& name == "invoke"
			{
				payload = Some(r#return.clone());
			}
			payload.is_some()
		});
		if payload.is_some() {
			break;
		}
	}
	payload
}

/// The `IntrinsicRequirements` struct records which intrinsics a shader calls, so each backend declares only the
/// helpers, extensions, and builtins the shader needs. Build it with [`intrinsic_requirements`].
#[derive(Default)]
pub(crate) struct IntrinsicRequirements {
	pub(crate) uses_atomic_compare_exchange: bool,
	pub(crate) uses_sincos: bool,
	pub(crate) uses_find_lsb: bool,
	pub(crate) uses_fma: bool,
	/// The shader converts to `f16` or a `vecNf16`.
	pub(crate) uses_f16: bool,
	pub(crate) uses_subgroup_intrinsics: bool,
	pub(crate) uses_simd_lane_id: bool,
	pub(crate) uses_downsample_min: bool,
	pub(crate) uses_downsample_max: bool,
	pub(crate) uses_render_target_array_index: bool,
	pub(crate) uses_intersect: bool,
	pub(crate) uses_trace_occlusion: bool,
}

/// Records the intrinsics that the code of every node in `order` calls, walking each body once.
///
/// It does not search the bodies of called functions, so pass every emitted function, as
/// [`crate::shader::generator::ordered_shader_nodes`] returns them.
fn intrinsic_requirements(order: &[besl::NodeReference]) -> IntrinsicRequirements {
	let mut requirements = IntrinsicRequirements::default();
	for node in order {
		any_code_node(node, false, &mut |node| {
			if let besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) = node.borrow().node()
				&& let Some(name) = intrinsic.borrow().get_name()
			{
				match name {
					"atomic_compare_exchange" => requirements.uses_atomic_compare_exchange = true,
					"sincos" => requirements.uses_sincos = true,
					"find_lsb" => requirements.uses_find_lsb = true,
					"fma" => requirements.uses_fma = true,
					"f16" | "vec2f16" | "vec3f16" | "vec4f16" => requirements.uses_f16 = true,
					"subgroup_lane_index" => {
						requirements.uses_subgroup_intrinsics = true;
						requirements.uses_simd_lane_id = true;
					}
					name if SUBGROUP_INTRINSICS.contains(&name) => requirements.uses_subgroup_intrinsics = true,
					"downsample_min" => requirements.uses_downsample_min = true,
					"downsample_max" => requirements.uses_downsample_max = true,
					"set_mesh_primitive_render_target_array_index" => requirements.uses_render_target_array_index = true,
					"intersect" => requirements.uses_intersect = true,
					"trace_occlusion" => requirements.uses_trace_occlusion = true,
					_ => {}
				}
			}
			false
		});
	}
	requirements
}
