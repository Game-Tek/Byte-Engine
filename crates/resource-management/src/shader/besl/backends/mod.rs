pub mod glsl;
pub mod hlsl;
pub mod msl;
pub mod platform;
pub mod spirv;

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
		return (lod + nested) * f32(size.x);
	}
	main: fn (input: StageInput, pipeline_input: interface { index: u32, uv: vec2f }) -> output { color: vec4f } {
		let color: vec4f = shade(pipeline_input.index, pipeline_input.uv);
		return { color };
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
const SUBGROUP_INTRINSICS: [&str; 8] = [
	"subgroup_lane_index",
	"subgroup_ballot",
	"subgroup_ballot_any",
	"subgroup_ballot_find_lsb",
	"subgroup_ballot_count",
	"subgroup_ballot_and_not",
	"subgroup_broadcast_u32",
	"subgroup_broadcast_f32",
];

/// Reports whether any node in `order` uses one of BESL's compute-only subgroup operations.
fn uses_subgroup_intrinsics(order: &[besl::NodeReference]) -> bool {
	order.iter().any(|node| {
		any_code_node(node, false, &mut |node| {
			SUBGROUP_INTRINSICS.iter().any(|intrinsic| is_intrinsic_call(node, intrinsic))
		})
	})
}
