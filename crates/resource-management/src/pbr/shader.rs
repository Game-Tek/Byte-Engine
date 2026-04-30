use std::fmt::{Display, Formatter};

use super::{BrdfMaterialDescription, BrdfMaterialValidationError, BrdfMetallicRoughness, BrdfNode, BrdfNodeId, BrdfValue};

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
			besl::parser::Node::let_assignment("albedo", "vec4f", vector4_expression(base_color)),
			besl::parser::Node::let_assignment("metalness", "f32", scalar_expression(metallic)),
			besl::parser::Node::let_assignment("roughness", "f32", scalar_expression(roughness)),
			besl::parser::Node::let_assignment("normal", "vec3f", vector3_expression([0.0, 0.0, 1.0])),
		]),
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
	besl::parser::Node::literal_expression(leak_float_literal(value))
}

fn vector3_expression(value: [f32; 3]) -> besl::parser::Node<'static> {
	besl::parser::Node::call(
		"vec3f",
		vec![
			scalar_expression(value[0]),
			scalar_expression(value[1]),
			scalar_expression(value[2]),
		],
	)
}

fn vector4_expression(value: [f32; 4]) -> besl::parser::Node<'static> {
	besl::parser::Node::call(
		"vec4f",
		vec![
			scalar_expression(value[0]),
			scalar_expression(value[1]),
			scalar_expression(value[2]),
			scalar_expression(value[3]),
		],
	)
}

fn leak_float_literal(value: f32) -> &'static str {
	let mut literal = value.to_string();
	if !literal.contains('.') && !literal.contains('e') && !literal.contains('E') {
		literal.push_str(".0");
	}
	Box::leak(literal.into_boxed_str())
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
		besl::lex(program).expect("generated BESL program should lex");
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
		let besl::parser::Nodes::Scope { children, .. } = program.node() else {
			panic!("Expected root scope");
		};
		let main = children.iter().find(|child| child.name() == Some("main")).unwrap();
		let besl::parser::Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected main function");
		};

		assert_eq!(statements.len(), names.len());
		for (statement, name) in statements.iter().zip(names.iter()) {
			let besl::parser::Nodes::Expression(besl::parser::Expressions::Operator {
				name: operator, left, ..
			}) = statement.node()
			else {
				panic!("Expected assignment statement");
			};
			assert_eq!(*operator, "=");
			assert!(
				matches!(left.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::VariableDeclaration { name: member, .. }) if member == name)
			);
		}
	}

	fn main_statement<'a>(program: &'a besl::parser::Node<'a>, index: usize) -> &'a besl::parser::Node<'a> {
		let besl::parser::Nodes::Scope { children, .. } = program.node() else {
			panic!("Expected root scope");
		};
		let main = children.iter().find(|child| child.name() == Some("main")).unwrap();
		let besl::parser::Nodes::Function { statements, .. } = main.node() else {
			panic!("Expected main function");
		};
		&statements[index]
	}

	fn assignment_right<'a>(statement: &'a besl::parser::Node<'a>) -> &'a besl::parser::Node<'a> {
		let besl::parser::Nodes::Expression(besl::parser::Expressions::Operator { right, .. }) = statement.node() else {
			panic!("Expected assignment statement");
		};
		right
	}

	fn assert_vec4_call(node: &besl::parser::Node<'_>, expected: &[&str; 4]) {
		let besl::parser::Nodes::Expression(besl::parser::Expressions::Call { name, parameters }) = node.node() else {
			panic!("Expected vec4f call");
		};
		assert_eq!(*name, "vec4f");
		assert_eq!(parameters.len(), 4);

		for (parameter, expected) in parameters.iter().zip(expected.iter()) {
			assert!(
				matches!(parameter.node(), besl::parser::Nodes::Expression(besl::parser::Expressions::Literal { value }) if value == expected)
			);
		}
	}
}
