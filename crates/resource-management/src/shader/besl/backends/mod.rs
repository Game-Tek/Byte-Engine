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
