use std::fmt::{Display, Formatter};

use super::{
	material_texture_variable_name, BrdfChannel, BrdfMaterialDescription, BrdfMaterialValidationError, BrdfMetallicRoughness,
	BrdfNode, BrdfNodeId, BrdfTexture, BrdfValue,
};

/// Generates a BESL program from a solid-value BRDF material graph.
pub fn generate_solid_brdf_program(
	material: &BrdfMaterialDescription,
) -> Result<besl::parser::Node<'static>, BrdfShaderGenerationError> {
	let surface = solid_metallic_roughness_surface(material)?;
	let base_color = evaluate_solid_value(material, surface.base_color)?;
	let metallic = evaluate_solid_value(material, surface.metallic)?;
	let roughness = evaluate_solid_value(material, surface.roughness)?;

	let base_color = expect_vector4(surface.base_color, base_color)?;
	let metallic = expect_scalar(surface.metallic, metallic)?;
	let roughness = expect_scalar(surface.roughness, roughness)?;

	Ok(besl::parser::Node::root_with_children(vec![
		besl::parser::Node::main_function(vec![
			besl::parser::Node::member_assignment("albedo", vector4_expression(base_color)),
			besl::parser::Node::member_assignment("metalness", scalar_expression(metallic)),
			besl::parser::Node::member_assignment("roughness", scalar_expression(roughness)),
			besl::parser::Node::member_assignment("normal", vector3_expression([0.0, 0.0, 1.0])),
		]),
	]))
}

/// Generates a BESL program from a BRDF material graph that may sample visibility-pipeline material textures.
pub fn generate_textured_brdf_program(
	material: &BrdfMaterialDescription,
) -> Result<besl::parser::Node<'static>, BrdfShaderGenerationError> {
	let surface = solid_metallic_roughness_surface(material)?;
	let mut statements = Vec::with_capacity(12);
	let mut texture_samples = MaterialTextureSamples::new();

	let albedo = vector4_value_expression(&mut texture_samples, material, surface.base_color)?;
	append_textured_assignment(&mut statements, &mut texture_samples, "albedo", albedo);
	let metalness = scalar_value_expression(&mut texture_samples, material, surface.metallic)?;
	append_textured_assignment(&mut statements, &mut texture_samples, "metalness", metalness);
	let roughness = scalar_value_expression(&mut texture_samples, material, surface.roughness)?;
	append_textured_assignment(&mut statements, &mut texture_samples, "roughness", roughness);

	if let Some(normal) = surface.normal {
		let normal = vector3_value_expression(&mut texture_samples, material, normal)?;
		append_textured_assignment(&mut statements, &mut texture_samples, "normal", normal);
	} else {
		append_textured_assignment(
			&mut statements,
			&mut texture_samples,
			"normal",
			vector3_expression([0.0, 0.0, 1.0]),
		);
	}

	if let Some(occlusion) = surface.occlusion {
		let occlusion = scalar_value_expression(&mut texture_samples, material, occlusion)?;
		append_textured_assignment(&mut statements, &mut texture_samples, "occlusion", occlusion);
	}

	if let Some(emission) = surface.emission {
		let emission = vector3_value_expression(&mut texture_samples, material, emission)?;
		append_textured_assignment(&mut statements, &mut texture_samples, "emission", emission);
	}

	Ok(besl::parser::Node::root_with_children(vec![
		besl::parser::Node::main_function(statements),
	]))
}

fn solid_metallic_roughness_surface(
	material: &BrdfMaterialDescription,
) -> Result<BrdfMetallicRoughness, BrdfShaderGenerationError> {
	material.validate().map_err(BrdfShaderGenerationError::InvalidMaterial)?;

	match material.node(material.surface) {
		Ok(BrdfNode::MetallicRoughness(surface)) => Ok(*surface),
		Ok(_) => Err(BrdfShaderGenerationError::SurfaceNodeMustBeMetallicRoughness),
		Err(error) => Err(BrdfShaderGenerationError::InvalidMaterial(error)),
	}
}

fn evaluate_solid_value(material: &BrdfMaterialDescription, node: BrdfNodeId) -> Result<BrdfValue, BrdfShaderGenerationError> {
	match material.node(node).map_err(BrdfShaderGenerationError::InvalidMaterial)? {
		BrdfNode::Constant(value) => Ok(*value),
		BrdfNode::Multiply { left, right } => {
			let left_value = evaluate_solid_value(material, *left)?;
			let right_value = evaluate_solid_value(material, *right)?;
			multiply_values(*left, left_value, *right, right_value)
		}
		BrdfNode::Texture(_)
		| BrdfNode::ExtractChannel { .. }
		| BrdfNode::NormalMap { .. }
		| BrdfNode::Occlusion { .. }
		| BrdfNode::Emission { .. } => Err(BrdfShaderGenerationError::UnsupportedNode { node }),
		BrdfNode::MetallicRoughness(_) => Err(BrdfShaderGenerationError::InvalidNodeType { node }),
	}
}

fn value_expression(
	texture_samples: &mut MaterialTextureSamples,
	material: &BrdfMaterialDescription,
	node: BrdfNodeId,
) -> Result<besl::parser::Node<'static>, BrdfShaderGenerationError> {
	match material.node(node).map_err(BrdfShaderGenerationError::InvalidMaterial)? {
		BrdfNode::Constant(value) => Ok(brdf_value_expression(*value)),
		BrdfNode::Texture(texture) => Ok(texture_sample_expression(texture_samples, *texture)),
		BrdfNode::Multiply { left, right } => {
			let left = value_expression(texture_samples, material, *left)?;
			let right = value_expression(texture_samples, material, *right)?;
			Ok(besl::parser::Node::operator("*", left, right))
		}
		BrdfNode::ExtractChannel { source, channel } => {
			let source = value_expression(texture_samples, material, *source)?;
			Ok(channel_expression(source, *channel))
		}
		BrdfNode::NormalMap { source, scale } => normal_map_expression(texture_samples, material, *source, *scale),
		BrdfNode::Occlusion { source, strength } => occlusion_expression(texture_samples, material, *source, *strength),
		BrdfNode::Emission { color } => vector3_value_expression(texture_samples, material, *color),
		BrdfNode::MetallicRoughness(_) => Err(BrdfShaderGenerationError::InvalidNodeType { node }),
	}
}

fn scalar_value_expression(
	texture_samples: &mut MaterialTextureSamples,
	material: &BrdfMaterialDescription,
	node: BrdfNodeId,
) -> Result<besl::parser::Node<'static>, BrdfShaderGenerationError> {
	Ok(
		match material.node(node).map_err(BrdfShaderGenerationError::InvalidMaterial)? {
			BrdfNode::Constant(BrdfValue::Scalar(value)) => scalar_expression(*value),
			BrdfNode::Constant(_) => return Err(BrdfShaderGenerationError::InvalidNodeType { node }),
			_ => value_expression(texture_samples, material, node)?,
		},
	)
}

fn vector3_value_expression(
	texture_samples: &mut MaterialTextureSamples,
	material: &BrdfMaterialDescription,
	node: BrdfNodeId,
) -> Result<besl::parser::Node<'static>, BrdfShaderGenerationError> {
	Ok(
		match material.node(node).map_err(BrdfShaderGenerationError::InvalidMaterial)? {
			BrdfNode::Constant(BrdfValue::Vector3(value)) => vector3_expression(*value),
			BrdfNode::Texture(texture) => texture_rgb_expression(texture_samples, *texture),
			BrdfNode::Multiply { left, right } => {
				let left = vector3_factor_expression(texture_samples, material, *left)?;
				let right = vector3_factor_expression(texture_samples, material, *right)?;
				besl::parser::Node::operator("*", left, right)
			}
			BrdfNode::Emission { color } => vector3_value_expression(texture_samples, material, *color)?,
			BrdfNode::Constant(_) => return Err(BrdfShaderGenerationError::InvalidNodeType { node }),
			_ => value_expression(texture_samples, material, node)?,
		},
	)
}

fn vector3_factor_expression(
	texture_samples: &mut MaterialTextureSamples,
	material: &BrdfMaterialDescription,
	node: BrdfNodeId,
) -> Result<besl::parser::Node<'static>, BrdfShaderGenerationError> {
	Ok(
		match material.node(node).map_err(BrdfShaderGenerationError::InvalidMaterial)? {
			BrdfNode::Constant(BrdfValue::Scalar(value)) => scalar_expression(*value),
			BrdfNode::Constant(BrdfValue::Vector3(value)) => vector3_expression(*value),
			BrdfNode::Texture(texture) => texture_rgb_expression(texture_samples, *texture),
			BrdfNode::ExtractChannel { source, channel } => {
				let source = value_expression(texture_samples, material, *source)?;
				channel_expression(source, *channel)
			}
			BrdfNode::Multiply { left, right } => {
				let left = vector3_factor_expression(texture_samples, material, *left)?;
				let right = vector3_factor_expression(texture_samples, material, *right)?;
				besl::parser::Node::operator("*", left, right)
			}
			_ => vector3_value_expression(texture_samples, material, node)?,
		},
	)
}

fn vector4_value_expression(
	texture_samples: &mut MaterialTextureSamples,
	material: &BrdfMaterialDescription,
	node: BrdfNodeId,
) -> Result<besl::parser::Node<'static>, BrdfShaderGenerationError> {
	Ok(
		match material.node(node).map_err(BrdfShaderGenerationError::InvalidMaterial)? {
			BrdfNode::Constant(BrdfValue::Vector4(value)) => vector4_expression(*value),
			BrdfNode::Constant(_) => return Err(BrdfShaderGenerationError::InvalidNodeType { node }),
			_ => value_expression(texture_samples, material, node)?,
		},
	)
}

fn brdf_value_expression(value: BrdfValue) -> besl::parser::Node<'static> {
	match value {
		BrdfValue::Scalar(value) => scalar_expression(value),
		BrdfValue::Vector3(value) => vector3_expression(value),
		BrdfValue::Vector4(value) => vector4_expression(value),
	}
}

/// The `MaterialTextureSamples` struct reuses raw material texels within one generated BRDF entry point.
///
/// Bindings are emitted immediately before their first consumer so a texel's source-level lifetime starts as late as possible.
struct MaterialTextureSamples {
	entries: Vec<MaterialTextureSample>,
	emitted_entries: usize,
}

/// The `MaterialTextureSample` struct identifies one reusable raw material texel local.
struct MaterialTextureSample {
	texture: BrdfTexture,
	local_name: String,
}

impl MaterialTextureSamples {
	fn new() -> Self {
		Self {
			entries: Vec::with_capacity(8),
			emitted_entries: 0,
		}
	}

	fn sample(&mut self, texture: BrdfTexture) -> besl::parser::Node<'static> {
		if let Some(entry) = self.entries.iter().find(|entry| entry.texture == texture) {
			return besl::parser::Node::member_expression(entry.local_name.clone());
		}

		// BrdfTexture equality includes texcoord_channel. Do not collapse samples by image alone: future UV lowering must keep
		// distinct texture-coordinate sources independent.
		let local_name = format!("material_texture_sample_{}", self.entries.len());
		self.entries.push(MaterialTextureSample {
			texture,
			local_name: local_name.clone(),
		});
		besl::parser::Node::member_expression(local_name)
	}

	fn append_new_bindings(&mut self, statements: &mut Vec<besl::parser::Node<'static>>) {
		// A local must be declared before the first material property that references it.
		for entry in &self.entries[self.emitted_entries..] {
			statements.push(besl::parser::Node::let_assignment(
				entry.local_name.clone(),
				"vec4f16",
				besl::parser::Node::call("vec4f16", vec![material_texture_sample_expression(entry.texture)]),
			));
		}
		self.emitted_entries = self.entries.len();
	}
}

/// Emits newly discovered texture locals immediately before the material property that first uses them.
fn append_textured_assignment(
	statements: &mut Vec<besl::parser::Node<'static>>,
	texture_samples: &mut MaterialTextureSamples,
	target: &'static str,
	value: besl::parser::Node<'static>,
) {
	texture_samples.append_new_bindings(statements);
	statements.push(besl::parser::Node::member_assignment(target, value));
}

fn material_texture_sample_expression(texture: BrdfTexture) -> besl::parser::Node<'static> {
	// The visibility material transform maps each generated Texture2D variable to a per-material slot.
	// Keep the BRDF graph independent from final bindless descriptor indices by referring to the generated variable name.
	besl::parser::Node::call(
		"sample_material",
		vec![besl::parser::Node::member_expression(material_texture_variable_name(
			texture.image_index,
		))],
	)
}

fn texture_sample_expression(
	texture_samples: &mut MaterialTextureSamples,
	texture: BrdfTexture,
) -> besl::parser::Node<'static> {
	texture_samples.sample(texture)
}

fn texture_rgb_expression(texture_samples: &mut MaterialTextureSamples, texture: BrdfTexture) -> besl::parser::Node<'static> {
	let sample = texture_sample_expression(texture_samples, texture);
	besl::parser::Node::call(
		"vec3f16",
		vec![
			channel_expression(sample.clone(), BrdfChannel::Red),
			channel_expression(sample.clone(), BrdfChannel::Green),
			channel_expression(sample, BrdfChannel::Blue),
		],
	)
}

fn normal_map_expression(
	texture_samples: &mut MaterialTextureSamples,
	material: &BrdfMaterialDescription,
	source: BrdfNodeId,
	scale: f32,
) -> Result<besl::parser::Node<'static>, BrdfShaderGenerationError> {
	let source = match material.node(source).map_err(BrdfShaderGenerationError::InvalidMaterial)? {
		BrdfNode::Texture(texture) => decode_material_normal_expression(texture_sample_expression(texture_samples, *texture)),
		_ => vector3_value_expression(texture_samples, material, source)?,
	};

	if scale == 1.0 {
		Ok(source)
	} else {
		Ok(besl::parser::Node::call(
			"scale_material_normal_xy_f16",
			vec![source, scalar_expression(scale)],
		))
	}
}

fn occlusion_expression(
	texture_samples: &mut MaterialTextureSamples,
	material: &BrdfMaterialDescription,
	source: BrdfNodeId,
	strength: f32,
) -> Result<besl::parser::Node<'static>, BrdfShaderGenerationError> {
	let source = channel_expression(value_expression(texture_samples, material, source)?, BrdfChannel::Red);
	if strength == 1.0 {
		Ok(source)
	} else {
		// glTF occlusion strength blends from no occlusion at 1.0 toward the sampled occlusion channel.
		Ok(besl::parser::Node::operator(
			"+",
			scalar_expression(1.0 - strength),
			besl::parser::Node::operator("*", scalar_expression(strength), source),
		))
	}
}

fn decode_material_normal_expression(sample: besl::parser::Node<'static>) -> besl::parser::Node<'static> {
	besl::parser::Node::call("decode_material_normal_f16", vec![sample])
}

fn channel_expression(value: besl::parser::Node<'static>, channel: BrdfChannel) -> besl::parser::Node<'static> {
	besl::parser::Node::accessor(value, besl::parser::Node::member_expression(channel_member(channel)))
}

fn channel_member(channel: BrdfChannel) -> &'static str {
	match channel {
		BrdfChannel::Red => "x",
		BrdfChannel::Green => "y",
		BrdfChannel::Blue => "z",
		BrdfChannel::Alpha => "w",
	}
}

fn multiply_values(
	left: BrdfNodeId,
	left_value: BrdfValue,
	right: BrdfNodeId,
	right_value: BrdfValue,
) -> Result<BrdfValue, BrdfShaderGenerationError> {
	match (left_value, right_value) {
		(BrdfValue::Scalar(left), BrdfValue::Scalar(right)) => Ok(BrdfValue::Scalar(left * right)),
		(BrdfValue::Scalar(left), BrdfValue::Vector3(right)) => Ok(BrdfValue::Vector3(scale_vector3(right, left))),
		(BrdfValue::Vector3(left), BrdfValue::Scalar(right)) => Ok(BrdfValue::Vector3(scale_vector3(left, right))),
		(BrdfValue::Scalar(left), BrdfValue::Vector4(right)) => Ok(BrdfValue::Vector4(scale_vector4(right, left))),
		(BrdfValue::Vector4(left), BrdfValue::Scalar(right)) => Ok(BrdfValue::Vector4(scale_vector4(left, right))),
		(BrdfValue::Vector3(left), BrdfValue::Vector3(right)) => Ok(BrdfValue::Vector3(multiply_vector3(left, right))),
		(BrdfValue::Vector4(left), BrdfValue::Vector4(right)) => Ok(BrdfValue::Vector4(multiply_vector4(left, right))),
		_ => Err(BrdfShaderGenerationError::TypeMismatch { left, right }),
	}
}

fn scale_vector3(value: [f32; 3], scalar: f32) -> [f32; 3] {
	[value[0] * scalar, value[1] * scalar, value[2] * scalar]
}

fn scale_vector4(value: [f32; 4], scalar: f32) -> [f32; 4] {
	[value[0] * scalar, value[1] * scalar, value[2] * scalar, value[3] * scalar]
}

fn multiply_vector3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	[left[0] * right[0], left[1] * right[1], left[2] * right[2]]
}

fn multiply_vector4(left: [f32; 4], right: [f32; 4]) -> [f32; 4] {
	[left[0] * right[0], left[1] * right[1], left[2] * right[2], left[3] * right[3]]
}

fn expect_scalar(node: BrdfNodeId, value: BrdfValue) -> Result<f32, BrdfShaderGenerationError> {
	match value {
		BrdfValue::Scalar(value) => Ok(value),
		_ => Err(BrdfShaderGenerationError::InvalidNodeType { node }),
	}
}

fn expect_vector4(node: BrdfNodeId, value: BrdfValue) -> Result<[f32; 4], BrdfShaderGenerationError> {
	match value {
		BrdfValue::Vector4(value) => Ok(value),
		_ => Err(BrdfShaderGenerationError::InvalidNodeType { node }),
	}
}

fn scalar_expression(value: f32) -> besl::parser::Node<'static> {
	besl::parser::Node::call("f16", vec![besl::parser::Node::literal_expression(float_literal(value))])
}

fn vector3_expression(value: [f32; 3]) -> besl::parser::Node<'static> {
	besl::parser::Node::call(
		"vec3f16",
		vec![
			scalar_expression(value[0]),
			scalar_expression(value[1]),
			scalar_expression(value[2]),
		],
	)
}

fn vector4_expression(value: [f32; 4]) -> besl::parser::Node<'static> {
	besl::parser::Node::call(
		"vec4f16",
		vec![
			scalar_expression(value[0]),
			scalar_expression(value[1]),
			scalar_expression(value[2]),
			scalar_expression(value[3]),
		],
	)
}

fn float_literal(value: f32) -> String {
	let mut literal = value.to_string();
	if !literal.contains('.') && !literal.contains('e') && !literal.contains('E') {
		literal.push_str(".0");
	}
	literal
}

/// The `BrdfShaderGenerationError` enum explains why a BRDF graph cannot be lowered into a solid BESL program.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrdfShaderGenerationError {
	InvalidMaterial(BrdfMaterialValidationError),
	SurfaceNodeMustBeMetallicRoughness,
	UnsupportedNode { node: BrdfNodeId },
	InvalidNodeType { node: BrdfNodeId },
	TypeMismatch { left: BrdfNodeId, right: BrdfNodeId },
}

impl Display for BrdfShaderGenerationError {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		match self {
			BrdfShaderGenerationError::InvalidMaterial(_) => write!(
				f,
				"Invalid BRDF material graph. The most likely cause is an importer producing dangling node references."
			),
			BrdfShaderGenerationError::SurfaceNodeMustBeMetallicRoughness => write!(
				f,
				"Unsupported BRDF surface node. The most likely cause is trying to generate a shader for a non metallic-roughness graph."
			),
			BrdfShaderGenerationError::UnsupportedNode { .. } => write!(
				f,
				"Unsupported BRDF node. The most likely cause is trying to generate a solid shader from a graph that needs textures."
			),
			BrdfShaderGenerationError::InvalidNodeType { .. } => write!(
				f,
				"Invalid BRDF node type. The most likely cause is a material property referencing a node with the wrong value type."
			),
			BrdfShaderGenerationError::TypeMismatch { .. } => write!(
				f,
				"BRDF node type mismatch. The most likely cause is multiplying incompatible material value types."
			),
		}
	}
}

impl std::error::Error for BrdfShaderGenerationError {}

#[cfg(test)]
mod tests {
	use besl::vm::{f16, output_slot, Buffer, DescriptorBindings, ExecutableProgram, ResourceSlot, Texture, Value};

	use super::*;
	use crate::pbr::{BrdfAlphaMode, BrdfMaterialBuilder, BrdfMetallicRoughness, BrdfTexture};

	#[test]
	fn generates_besl_program_for_constant_material() {
		let material = test_material(
			BrdfValue::Vector4([0.2, 0.3, 0.4, 1.0]),
			BrdfValue::Scalar(0.7),
			BrdfValue::Scalar(0.8),
		);

		let program = generate_solid_brdf_program(&material).expect("material should generate");

		assert_main_assignment_order(&program, &["albedo", "metalness", "roughness", "normal"]);
	}

	/// Verifies the constant-material generator produces the expected BRDF values when executed.
	#[test]
	fn constant_material_besl_program_runs_in_the_vm() {
		let material = test_material(
			BrdfValue::Vector4([0.2, 0.3, 0.4, 0.9]),
			BrdfValue::Scalar(0.7),
			BrdfValue::Scalar(0.8),
		);
		let mut program = generate_solid_brdf_program(&material)
			.expect("Failed to generate constant material BESL. The most likely cause is an invalid BRDF material graph.");
		program.add(vec![
			besl::parser::Node::output("albedo", "vec4f16", 0),
			besl::parser::Node::output("metalness", "f16", 1),
			besl::parser::Node::output("roughness", "f16", 2),
			besl::parser::Node::output("normal", "vec3f16", 3),
		]);
		let program = besl::lex(program)
			.expect("Failed to lex constant material BESL. The most likely cause is an invalid generated material program.");
		let executable = ExecutableProgram::compile(program)
			.expect("Failed to compile constant material BESL for the VM. The most likely cause is missing VM shader support.");
		let mut outputs: [Buffer; 4] = std::array::from_fn(|location| {
			Buffer::new(
				executable
					.output_layout(location as u8)
					.expect("Missing material output layout. The most likely cause is an unresolved generated assignment.")
					.clone(),
			)
		});
		{
			let mut descriptors = DescriptorBindings::new();
			for (location, output) in outputs.iter_mut().enumerate() {
				descriptors.bind_buffer(output_slot(location as u8), output);
			}
			executable
				.run_main(&mut descriptors)
				.expect("Failed to execute constant material BESL. The most likely cause is incomplete BESL VM support.");
		}

		assert_eq!(
			outputs[0].read("albedo").expect("Expected albedo output"),
			Value::Vec4F16([f16::from_f32(0.2), f16::from_f32(0.3), f16::from_f32(0.4), f16::from_f32(0.9)])
		);
		assert_eq!(
			outputs[1].read("metalness").expect("Expected metalness output"),
			Value::F16(f16::from_f32(0.7))
		);
		assert_eq!(
			outputs[2].read("roughness").expect("Expected roughness output"),
			Value::F16(f16::from_f32(0.8))
		);
		assert_eq!(
			outputs[3].read("normal").expect("Expected normal output"),
			Value::Vec3F16([f16::from_f32(0.0), f16::from_f32(0.0), f16::from_f32(1.0)])
		);
	}

	/// Verifies the textured-material generator samples every BRDF image role and extracts packed channels when executed.
	#[test]
	fn textured_material_besl_program_runs_in_the_vm() {
		let mut builder = BrdfMaterialBuilder::new();
		let base_color = builder.texture(BrdfTexture {
			image_index: 3,
			texcoord_channel: 0,
		});
		let metallic_roughness = builder.texture(BrdfTexture {
			image_index: 4,
			texcoord_channel: 0,
		});
		let metallic = builder.extract_channel(metallic_roughness, BrdfChannel::Blue);
		let roughness = builder.extract_channel(metallic_roughness, BrdfChannel::Green);
		let normal_source = builder.texture(BrdfTexture {
			image_index: 5,
			texcoord_channel: 0,
		});
		let normal = builder.add(BrdfNode::NormalMap {
			source: normal_source,
			scale: 0.5,
		});
		let occlusion_source = builder.texture(BrdfTexture {
			image_index: 6,
			texcoord_channel: 0,
		});
		let occlusion = builder.add(BrdfNode::Occlusion {
			source: occlusion_source,
			strength: 1.0,
		});
		let emission_source = builder.texture(BrdfTexture {
			image_index: 7,
			texcoord_channel: 0,
		});
		let emission = builder.add(BrdfNode::Emission { color: emission_source });
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: Some(normal),
			occlusion: Some(occlusion),
			emission: Some(emission),
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);
		let mut program = generate_textured_brdf_program(&material)
			.expect("Failed to generate textured material BESL. The most likely cause is an invalid BRDF material graph.");
		// Production resolves these logical material slots through a bindless table; the VM adapter maps the same slots to fixed descriptors.
		let mut helpers = besl::parse(
			r#"
			sample_material: fn (slot: u32) -> vec4f {
				if (slot == 3) { return sample(base_color_texture, vec2f(0.5, 0.5)); }
				if (slot == 4) { return sample(metallic_roughness_texture, vec2f(0.5, 0.5)); }
				if (slot == 5) { return sample(normal_texture, vec2f(0.5, 0.5)); }
				if (slot == 6) { return sample(occlusion_texture, vec2f(0.5, 0.5)); }
				return sample(emission_texture, vec2f(0.5, 0.5));
			}

			decode_material_normal_f16: fn (encoded: vec4f16) -> vec3f16 {
				return vec3f16(encoded.x, encoded.y, encoded.z);
			}

			scale_material_normal_xy_f16: fn (normal: vec3f16, scale: f16) -> vec3f16 {
				return vec3f16(normal.x * scale, normal.y * scale, normal.z);
			}
			"#,
		)
		.expect("Failed to parse the material sampling adapter. The most likely cause is invalid BESL test syntax.");
		let helper_functions = match helpers.node_mut() {
			besl::parser::Nodes::Scope { children, .. } => std::mem::take(children),
			_ => panic!("Invalid material sampling adapter. The most likely cause is an unexpected BESL parser root."),
		};
		program.add(vec![
			besl::parser::Node::binding(
				"base_color_texture",
				besl::parser::Node::combined_image_sampler(),
				3,
				true,
				false,
			),
			besl::parser::Node::binding(
				"metallic_roughness_texture",
				besl::parser::Node::combined_image_sampler(),
				4,
				true,
				false,
			),
			besl::parser::Node::binding("normal_texture", besl::parser::Node::combined_image_sampler(), 5, true, false),
			besl::parser::Node::binding(
				"occlusion_texture",
				besl::parser::Node::combined_image_sampler(),
				6,
				true,
				false,
			),
			besl::parser::Node::binding(
				"emission_texture",
				besl::parser::Node::combined_image_sampler(),
				7,
				true,
				false,
			),
			besl::parser::Node::constant("material_texture_3", "u32", besl::parser::Node::literal_expression("3")),
			besl::parser::Node::constant("material_texture_4", "u32", besl::parser::Node::literal_expression("4")),
			besl::parser::Node::constant("material_texture_5", "u32", besl::parser::Node::literal_expression("5")),
			besl::parser::Node::constant("material_texture_6", "u32", besl::parser::Node::literal_expression("6")),
			besl::parser::Node::constant("material_texture_7", "u32", besl::parser::Node::literal_expression("7")),
		]);
		program.add(helper_functions);
		program.add(vec![
			besl::parser::Node::output("albedo", "vec4f16", 0),
			besl::parser::Node::output("metalness", "f16", 1),
			besl::parser::Node::output("roughness", "f16", 2),
			besl::parser::Node::output("normal", "vec3f16", 3),
			besl::parser::Node::output("occlusion", "f16", 4),
			besl::parser::Node::output("emission", "vec3f16", 5),
		]);
		let program = besl::lex(program)
			.expect("Failed to lex textured material BESL. The most likely cause is an invalid generated material program.");
		let executable = ExecutableProgram::compile(program)
			.expect("Failed to compile textured material BESL for the VM. The most likely cause is missing VM shader support.");
		let mut outputs: [Buffer; 6] = std::array::from_fn(|location| {
			Buffer::new(
				executable
					.output_layout(location as u8)
					.expect("Missing textured material output layout. The most likely cause is an unresolved assignment.")
					.clone(),
			)
		});
		let mut base_color_texture = Texture::new(1, 1)
			.expect("Failed to create the base-color texture. The most likely cause is an invalid test extent.");
		base_color_texture
			.write([0, 0], [0.2, 0.4, 0.6, 0.8])
			.expect("Failed to seed the base-color texture. The most likely cause is an invalid texel coordinate.");
		let mut metallic_roughness_texture = Texture::new(1, 1)
			.expect("Failed to create the metallic-roughness texture. The most likely cause is an invalid test extent.");
		metallic_roughness_texture
			.write([0, 0], [1.0, 0.35, 0.7, 1.0])
			.expect("Failed to seed the metallic-roughness texture. The most likely cause is an invalid texel coordinate.");
		let mut normal_texture =
			Texture::new(1, 1).expect("Failed to create the normal texture. The most likely cause is an invalid test extent.");
		normal_texture
			.write([0, 0], [0.1, 0.2, 0.97, 1.0])
			.expect("Failed to seed the normal texture. The most likely cause is an invalid texel coordinate.");
		let mut occlusion_texture = Texture::new(1, 1)
			.expect("Failed to create the occlusion texture. The most likely cause is an invalid test extent.");
		occlusion_texture
			.write([0, 0], [0.45, 1.0, 1.0, 1.0])
			.expect("Failed to seed the occlusion texture. The most likely cause is an invalid texel coordinate.");
		let mut emission_texture = Texture::new(1, 1)
			.expect("Failed to create the emission texture. The most likely cause is an invalid test extent.");
		emission_texture
			.write([0, 0], [0.8, 0.3, 0.1, 1.0])
			.expect("Failed to seed the emission texture. The most likely cause is an invalid texel coordinate.");

		{
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(3), &mut base_color_texture);
			descriptors.bind_texture(ResourceSlot::new(4), &mut metallic_roughness_texture);
			descriptors.bind_texture(ResourceSlot::new(5), &mut normal_texture);
			descriptors.bind_texture(ResourceSlot::new(6), &mut occlusion_texture);
			descriptors.bind_texture(ResourceSlot::new(7), &mut emission_texture);
			for (location, output) in outputs.iter_mut().enumerate() {
				descriptors.bind_buffer(output_slot(location as u8), output);
			}
			executable
				.run_main(&mut descriptors)
				.expect("Failed to execute textured material BESL. The most likely cause is incomplete BESL VM support.");
		}

		assert_eq!(
			outputs[0].read("albedo").expect("Expected albedo output"),
			Value::Vec4F16([f16::from_f32(0.2), f16::from_f32(0.4), f16::from_f32(0.6), f16::from_f32(0.8)])
		);
		assert_eq!(
			outputs[1].read("metalness").expect("Expected metalness output"),
			Value::F16(f16::from_f32(0.7))
		);
		assert_eq!(
			outputs[2].read("roughness").expect("Expected roughness output"),
			Value::F16(f16::from_f32(0.35))
		);
		assert_eq!(
			outputs[3].read("normal").expect("Expected normal output"),
			Value::Vec3F16([f16::from_f32(0.05), f16::from_f32(0.1), f16::from_f32(0.97)])
		);
		assert_eq!(
			outputs[4].read("occlusion").expect("Expected occlusion output"),
			Value::F16(f16::from_f32(0.45))
		);
		assert_eq!(
			outputs[5].read("emission").expect("Expected emission output"),
			Value::Vec3F16([f16::from_f32(0.8), f16::from_f32(0.3), f16::from_f32(0.1)])
		);
	}

	#[test]
	fn folds_scalar_vector_multiplication() {
		let mut builder = BrdfMaterialBuilder::new();
		let factor = builder.constant(BrdfValue::Scalar(0.5));
		let color = builder.constant(BrdfValue::Vector4([0.2, 0.4, 0.6, 1.0]));
		let base_color = builder.multiply(factor, color);
		let metallic = builder.constant(BrdfValue::Scalar(0.25));
		let roughness = builder.constant(BrdfValue::Scalar(0.75));
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		let program = generate_solid_brdf_program(&material).expect("material should generate");

		let albedo_assignment = main_statement(&program, 0);
		let vector = assignment_right(albedo_assignment);
		assert_vec4_call(vector, &["0.1", "0.2", "0.3", "0.5"]);
	}

	#[test]
	fn folds_vector_vector_multiplication() {
		let mut builder = BrdfMaterialBuilder::new();
		let left = builder.constant(BrdfValue::Vector4([0.5, 0.5, 0.5, 0.5]));
		let right = builder.constant(BrdfValue::Vector4([0.2, 0.4, 0.6, 0.8]));
		let base_color = builder.multiply(left, right);
		let metallic = builder.constant(BrdfValue::Scalar(1.0));
		let roughness = builder.constant(BrdfValue::Scalar(1.0));
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		let program = generate_solid_brdf_program(&material).expect("material should generate");

		let vector = assignment_right(main_statement(&program, 0));
		assert_vec4_call(vector, &["0.1", "0.2", "0.3", "0.4"]);
	}

	#[test]
	fn rejects_texture_nodes() {
		let mut builder = BrdfMaterialBuilder::new();
		let base_color = builder.texture(BrdfTexture {
			image_index: 0,
			texcoord_channel: 0,
		});
		let metallic = builder.constant(BrdfValue::Scalar(1.0));
		let roughness = builder.constant(BrdfValue::Scalar(1.0));
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		assert!(matches!(
			generate_solid_brdf_program(&material),
			Err(BrdfShaderGenerationError::UnsupportedNode { node }) if node == base_color
		));
	}

	#[test]
	fn generates_textured_program_with_texture_samples() {
		let mut builder = BrdfMaterialBuilder::new();
		let base_color = builder.texture(BrdfTexture {
			image_index: 3,
			texcoord_channel: 0,
		});
		let metallic_roughness = builder.texture(BrdfTexture {
			image_index: 4,
			texcoord_channel: 0,
		});
		let metallic = builder.extract_channel(metallic_roughness, crate::pbr::BrdfChannel::Blue);
		let roughness = builder.extract_channel(metallic_roughness, crate::pbr::BrdfChannel::Green);
		let normal_source = builder.texture(BrdfTexture {
			image_index: 5,
			texcoord_channel: 0,
		});
		let normal = builder.add(BrdfNode::NormalMap {
			source: normal_source,
			scale: 1.0,
		});
		let occlusion_source = builder.texture(BrdfTexture {
			image_index: 6,
			texcoord_channel: 0,
		});
		let occlusion = builder.add(BrdfNode::Occlusion {
			source: occlusion_source,
			strength: 1.0,
		});
		let emission_source = builder.texture(BrdfTexture {
			image_index: 7,
			texcoord_channel: 0,
		});
		let emission = builder.add(BrdfNode::Emission { color: emission_source });
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: Some(normal),
			occlusion: Some(occlusion),
			emission: Some(emission),
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		let program = generate_textured_brdf_program(&material).expect("material should generate");

		assert_main_assignment_order(
			&program,
			&["albedo", "metalness", "roughness", "normal", "occlusion", "emission"],
		);
		let statements = main_statements(&program);
		assert_eq!(statements.len(), 11);
		assert_texture_sample_binding(&statements[0], "material_texture_sample_0", "material_texture_3");
		assert_texture_sample_binding(&statements[2], "material_texture_sample_1", "material_texture_4");
		assert_texture_sample_binding(&statements[5], "material_texture_sample_2", "material_texture_5");
		assert_texture_sample_binding(&statements[7], "material_texture_sample_3", "material_texture_6");
		assert_texture_sample_binding(&statements[9], "material_texture_sample_4", "material_texture_7");

		assert_member_expression(
			assignment_right(surface_assignment(&program, "albedo")),
			"material_texture_sample_0",
		);

		let metallic_source = assert_accessor_channel(assignment_right(surface_assignment(&program, "metalness")), "z");
		assert_member_expression(metallic_source, "material_texture_sample_1");

		let roughness_source = assert_accessor_channel(assignment_right(surface_assignment(&program, "roughness")), "y");
		assert_member_expression(roughness_source, "material_texture_sample_1");

		let normal = assert_call(
			assignment_right(surface_assignment(&program, "normal")),
			"decode_material_normal_f16",
		);
		assert_eq!(normal.len(), 1);
		assert_member_expression(&normal[0], "material_texture_sample_2");

		let occlusion_source = assert_accessor_channel(assignment_right(surface_assignment(&program, "occlusion")), "x");
		assert_member_expression(occlusion_source, "material_texture_sample_3");

		let emission = assignment_right(surface_assignment(&program, "emission"));
		let parameters = assert_call(emission, "vec3f16");
		assert_eq!(parameters.len(), 3);
		for (parameter, channel) in parameters.iter().zip(["x", "y", "z"]) {
			let source = assert_accessor_channel(parameter, channel);
			assert_member_expression(source, "material_texture_sample_4");
		}
	}

	/// Verifies one raw texel is shared by every BRDF role that reads the same texture-coordinate source.
	#[test]
	fn reuses_a_texture_sample_across_color_packed_and_normal_roles() {
		let mut builder = BrdfMaterialBuilder::new();
		let texture = builder.texture(BrdfTexture {
			image_index: 3,
			texcoord_channel: 0,
		});
		let metallic = builder.extract_channel(texture, BrdfChannel::Blue);
		let roughness = builder.extract_channel(texture, BrdfChannel::Green);
		let normal = builder.add(BrdfNode::NormalMap {
			source: texture,
			scale: 0.5,
		});
		let occlusion = builder.add(BrdfNode::Occlusion {
			source: texture,
			strength: 0.75,
		});
		let emission = builder.add(BrdfNode::Emission { color: texture });
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color: texture,
			metallic,
			roughness,
			normal: Some(normal),
			occlusion: Some(occlusion),
			emission: Some(emission),
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		let program = generate_textured_brdf_program(&material).expect("material should generate");

		let bindings = texture_sample_bindings(&program);
		assert_eq!(bindings.len(), 1);
		assert_texture_sample_binding(bindings[0], "material_texture_sample_0", "material_texture_3");

		for name in ["albedo", "metalness", "roughness", "normal", "occlusion", "emission"] {
			assert_eq!(
				count_calls(assignment_right(surface_assignment(&program, name)), "sample_material"),
				0,
				"{name} bypassed the cached material texel"
			);
		}

		let normal_parameters = assert_call(
			assignment_right(surface_assignment(&program, "normal")),
			"scale_material_normal_xy_f16",
		);
		assert_eq!(normal_parameters.len(), 2);
		let decoded_parameters = assert_call(&normal_parameters[0], "decode_material_normal_f16");
		assert_member_expression(&decoded_parameters[0], "material_texture_sample_0");
	}

	/// Verifies the cache preserves texture-coordinate identity even while the current material sampler uses one UV input.
	#[test]
	fn keeps_samples_for_distinct_texture_coordinate_channels_separate() {
		let mut builder = BrdfMaterialBuilder::new();
		let base_color = builder.texture(BrdfTexture {
			image_index: 3,
			texcoord_channel: 0,
		});
		let metallic_roughness = builder.texture(BrdfTexture {
			image_index: 3,
			texcoord_channel: 1,
		});
		let metallic = builder.extract_channel(metallic_roughness, BrdfChannel::Blue);
		let roughness = builder.extract_channel(metallic_roughness, BrdfChannel::Green);
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		let program = generate_textured_brdf_program(&material).expect("material should generate");

		let bindings = texture_sample_bindings(&program);
		assert_eq!(bindings.len(), 2);
		assert_texture_sample_binding(bindings[0], "material_texture_sample_0", "material_texture_3");
		assert_texture_sample_binding(bindings[1], "material_texture_sample_1", "material_texture_3");
	}

	#[test]
	fn rejects_extract_channel_nodes() {
		let mut builder = BrdfMaterialBuilder::new();
		let source = builder.constant(BrdfValue::Vector4([1.0, 1.0, 1.0, 1.0]));
		let base_color = builder.extract_channel(source, crate::pbr::BrdfChannel::Red);
		let metallic = builder.constant(BrdfValue::Scalar(1.0));
		let roughness = builder.constant(BrdfValue::Scalar(1.0));
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		assert!(matches!(
			generate_solid_brdf_program(&material),
			Err(BrdfShaderGenerationError::UnsupportedNode { node }) if node == base_color
		));
	}

	#[test]
	fn rejects_type_mismatch() {
		let mut builder = BrdfMaterialBuilder::new();
		let left = builder.constant(BrdfValue::Vector3([1.0, 1.0, 1.0]));
		let right = builder.constant(BrdfValue::Vector4([1.0, 1.0, 1.0, 1.0]));
		let base_color = builder.multiply(left, right);
		let metallic = builder.constant(BrdfValue::Scalar(1.0));
		let roughness = builder.constant(BrdfValue::Scalar(1.0));
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		let material = builder.finish(None, surface, false, BrdfAlphaMode::Opaque);

		assert!(matches!(
			generate_solid_brdf_program(&material),
			Err(BrdfShaderGenerationError::TypeMismatch { left: error_left, right: error_right }) if error_left == left && error_right == right
		));
	}

	#[test]
	fn rejects_invalid_graph() {
		let material = BrdfMaterialDescription {
			name: None,
			nodes: Vec::new(),
			surface: BrdfNodeId::new(0),
			double_sided: false,
			alpha_mode: BrdfAlphaMode::Opaque,
		};

		assert!(matches!(
			generate_solid_brdf_program(&material),
			Err(BrdfShaderGenerationError::InvalidMaterial(_))
		));
	}

	fn test_material(base_color: BrdfValue, metallic: BrdfValue, roughness: BrdfValue) -> BrdfMaterialDescription {
		let mut builder = BrdfMaterialBuilder::new();
		let base_color = builder.constant(base_color);
		let metallic = builder.constant(metallic);
		let roughness = builder.constant(roughness);
		let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
			base_color,
			metallic,
			roughness,
			normal: None,
			occlusion: None,
			emission: None,
		}));
		builder.finish(None, surface, false, BrdfAlphaMode::Opaque)
	}

	fn assert_main_assignment_order(program: &besl::parser::Node<'_>, names: &[&str]) {
		let assignments = main_statements(program)
			.iter()
			.filter(|statement| {
				matches!(
					statement.node(),
					besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { left, .. })
						if matches!(left.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Member { .. }))
				)
			})
			.collect::<Vec<_>>();

		assert_eq!(assignments.len(), names.len());
		for (statement, name) in assignments.into_iter().zip(names.iter()) {
			let besl::parser::Nodes::Expression(besl::parser::Expressions::Operator {
				name: operator, left, ..
			}) = statement.node()
			else {
				panic!("Expected assignment statement");
			};
			assert_eq!(*operator, "=");
			assert!(
				matches!(left.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name: member }) if member == name)
			);
		}
	}

	fn main_statements<'a>(program: &'a besl::parser::Node<'a>) -> &'a [besl::parser::Node<'a>] {
		let besl::parser::Nodes::Scope { children, .. } = program.node() else {
			panic!("Expected root scope");
		};
		let main = children.iter().find(|child| child.name() == Some("main")).unwrap();
		let besl::parser::Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected main function");
		};
		statements
	}

	fn main_statement<'a>(program: &'a besl::parser::Node<'a>, index: usize) -> &'a besl::parser::Node<'a> {
		&main_statements(program)[index]
	}

	fn surface_assignment<'a>(program: &'a besl::parser::Node<'a>, target: &str) -> &'a besl::parser::Node<'a> {
		main_statements(program)
			.iter()
			.find(|statement| {
				matches!(
					statement.node(),
					besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { left, .. })
						if matches!(left.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) if name == target)
				)
			})
			.unwrap_or_else(|| panic!("Expected `{target}` material assignment"))
	}

	fn texture_sample_bindings<'a>(program: &'a besl::parser::Node<'a>) -> Vec<&'a besl::parser::Node<'a>> {
		main_statements(program)
			.iter()
			.filter(|statement| {
				matches!(
					statement.node(),
					besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { left, .. })
						if matches!(left.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::VariableDeclaration { .. }))
				)
			})
			.collect()
	}

	fn assignment_right<'a>(statement: &'a besl::parser::Node<'a>) -> &'a besl::parser::Node<'a> {
		let besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { right, .. }) = statement.node() else {
			panic!("Expected assignment statement");
		};
		right
	}

	fn assert_sample_call(node: &besl::parser::Node<'_>, name: &str, variable: &str) {
		let parameters = assert_call(node, name);
		assert_eq!(parameters.len(), 1);
		assert_member_expression(&parameters[0], variable);
	}

	fn assert_texture_sample_binding(statement: &besl::parser::Node<'_>, local_name: &str, texture_slot: &str) {
		let besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { name, left, right }) = statement.node()
		else {
			panic!("Expected texture sample assignment");
		};
		assert_eq!(*name, "=");
		let besl::parser::Nodes::Expression(besl::parser::Expressions::VariableDeclaration { name, r#type }) = left.node()
		else {
			panic!("Expected texture sample local declaration");
		};
		assert_eq!(name.as_ref(), local_name);
		assert!(matches!(r#type, besl::parser::TypeName::Named(name) if *name == "vec4f16"));
		let sample = assert_call(right, "vec4f16");
		assert_eq!(sample.len(), 1);
		assert_sample_call(&sample[0], "sample_material", texture_slot);
	}

	fn assert_call<'a>(node: &'a besl::parser::Node<'a>, expected_name: &str) -> &'a [besl::parser::Node<'a>] {
		let besl::parser::Nodes::Expression(besl::parser::Expressions::Call { name, parameters, .. }) = node.node() else {
			panic!("Expected call expression");
		};
		assert!(matches!(name, besl::parser::TypeName::Named(name) if *name == expected_name));
		parameters
	}

	fn assert_accessor_channel<'a>(node: &'a besl::parser::Node<'a>, channel: &str) -> &'a besl::parser::Node<'a> {
		let besl::parser::Nodes::Expression(besl::parser::Expressions::Accessor { left, right }) = node.node() else {
			panic!("Expected accessor expression");
		};
		assert_member_expression(right, channel);
		left
	}

	fn count_calls(node: &besl::parser::Node<'_>, expected_name: &str) -> usize {
		match node.node() {
			besl::parser::Nodes::Expression(besl::parser::Expressions::Call { name, parameters, .. }) => {
				usize::from(matches!(name, besl::parser::TypeName::Named(name) if *name == expected_name))
					+ parameters
						.iter()
						.map(|parameter| count_calls(parameter, expected_name))
						.sum::<usize>()
			}
			besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { left, right, .. })
			| besl::parser::Nodes::Expression(besl::parser::Expressions::Accessor { left, right }) => {
				count_calls(left, expected_name) + count_calls(right, expected_name)
			}
			besl::parser::Nodes::Expression(besl::parser::Expressions::Expression(elements)) => {
				elements.iter().map(|element| count_calls(element, expected_name)).sum()
			}
			besl::parser::Nodes::Expression(besl::parser::Expressions::Return { value: Some(value) }) => {
				count_calls(value, expected_name)
			}
			besl::parser::Nodes::Expression(besl::parser::Expressions::Macro { body, .. }) => count_calls(body, expected_name),
			_ => 0,
		}
	}

	fn assert_member_expression(node: &besl::parser::Node<'_>, expected_name: &str) {
		let besl::parser::Nodes::Expression(besl::parser::Expressions::Member { name }) = node.node() else {
			panic!("Expected member expression");
		};
		assert_eq!(name.as_ref(), expected_name);
	}

	fn assert_vec4_call(node: &besl::parser::Node<'_>, expected: &[&str; 4]) {
		let besl::parser::Nodes::Expression(besl::parser::Expressions::Call { name, parameters, .. }) = node.node() else {
			panic!("Expected vec4f16 call");
		};
		assert!(matches!(name, besl::parser::TypeName::Named(name) if *name == "vec4f16"));
		assert_eq!(parameters.len(), 4);

		for (parameter, expected) in parameters.iter().zip(expected.iter()) {
			let literal = assert_call(parameter, "f16");
			assert_eq!(literal.len(), 1);
			assert!(
				matches!(literal[0].node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Literal { value }) if value == expected)
			);
		}
	}
}
