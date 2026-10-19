//! Converts glTF materials into BEMX graphs.

use super::graph::{BinaryOperator, GraphError, Node, NodeId, Surface, Type, Value};
use super::import::{ImportedGraphBuilder, ImportedMaterial};
use crate::types::AlphaMode;

/// The channel glTF stores roughness in, inside its metallic-roughness texture.
const ROUGHNESS_CHANNEL: u8 = 1;

/// The channel glTF stores metalness in, inside its metallic-roughness texture.
const METALNESS_CHANNEL: u8 = 2;

/// Converts a glTF material into a BEMX graph that shades the way the glTF metallic-roughness model describes.
///
/// Every factor multiplies its texture, as the glTF specification defines. The mesh importer stores texture
/// coordinate set zero only, so every texture samples that set whatever `texCoord` the material names.
pub(crate) fn material_from_gltf(material: &gltf::Material) -> Result<ImportedMaterial, GraphError> {
	let pbr = material.pbr_metallic_roughness();
	let mut graph = ImportedGraphBuilder::new();

	let base_color = graph.factor_times_image(
		Value::Vector4(pbr.base_color_factor()),
		pbr.base_color_texture().map(|texture| image_index(&texture.texture())),
	)?;
	let (rgb, opacity) = graph.color_and_opacity(base_color)?;

	let metallic_roughness = pbr
		.metallic_roughness_texture()
		.map(|texture| graph.image(image_index(&texture.texture()), Type::Vector4))
		.transpose()?;
	let metalness = factor_times_channel(&mut graph, pbr.metallic_factor(), metallic_roughness, METALNESS_CHANNEL)?;
	let roughness = factor_times_channel(&mut graph, pbr.roughness_factor(), metallic_roughness, ROUGHNESS_CHANNEL)?;

	let normal = material
		.normal_texture()
		.map(|texture| {
			let value = graph.image(image_index(&texture.texture()), Type::Vector3)?;
			let scale = graph.builder.constant(Value::Float(texture.scale()));

			graph.builder.add(Node::NormalMap { value, scale })
		})
		.transpose()?;

	let occlusion = material
		.occlusion_texture()
		.map(|texture| {
			let sample = graph.image(image_index(&texture.texture()), Type::Vector4)?;
			let occlusion = graph.builder.add(Node::Extract { value: sample, index: 0 })?;

			if texture.strength() == 1.0 {
				return Ok(occlusion);
			}

			// glTF occlusion strength blends from no occlusion at zero toward the sampled occlusion at one.
			let none = graph.builder.constant(Value::Float(1.0));
			let strength = graph.builder.constant(Value::Float(texture.strength()));

			graph.builder.add(Node::Mix {
				background: none,
				foreground: occlusion,
				factor: strength,
			})
		})
		.transpose()?;

	let emission = graph.factor_times_image(
		Value::Vector3(material.emissive_factor()),
		material.emissive_texture().map(|texture| image_index(&texture.texture())),
	)?;

	graph.finish(
		material.name().map(ToString::to_string),
		Surface {
			base_color: Some(rgb),
			opacity: Some(opacity),
			metalness: Some(metalness),
			roughness: Some(roughness),
			normal,
			emission: Some(emission),
			occlusion,
		},
		material.double_sided(),
		alpha_mode(material),
	)
}

/// Adds a scalar factor, multiplied by one channel of a texture when the material has one.
fn factor_times_channel(
	graph: &mut ImportedGraphBuilder,
	factor: f32,
	texture: Option<NodeId>,
	channel: u8,
) -> Result<NodeId, GraphError> {
	let factor = graph.builder.constant(Value::Float(factor));

	let Some(texture) = texture else {
		return Ok(factor);
	};

	let channel = graph.builder.add(Node::Extract {
		value: texture,
		index: channel,
	})?;

	graph.builder.add(Node::Binary {
		operator: BinaryOperator::Multiply,
		left: factor,
		right: channel,
	})
}

/// Returns the container image a glTF texture samples.
fn image_index(texture: &gltf::Texture) -> u32 {
	texture.source().index() as u32
}

fn alpha_mode(material: &gltf::Material) -> AlphaMode {
	match material.alpha_mode() {
		gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
		gltf::material::AlphaMode::Mask => AlphaMode::Mask(material.alpha_cutoff().unwrap_or(0.5)),
		gltf::material::AlphaMode::Blend => AlphaMode::Blend,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::bemx::{Graph, material_texture_slot};

	#[test]
	fn converts_gltf_scalar_vector_and_alpha_properties() {
		let gltf = parse_gltf(
			r#"{
				"asset":{"version":"2.0"},
				"materials":[{
					"name":"paint",
					"doubleSided":true,
					"alphaMode":"MASK",
					"alphaCutoff":0.25,
					"pbrMetallicRoughness":{
						"baseColorFactor":[0.25,0.5,0.75,0.8],
						"metallicFactor":0.4,
						"roughnessFactor":0.6
					},
					"emissiveFactor":[0.1,0.2,0.3]
				}]
			}"#,
		);

		let material = material_from_gltf(&gltf.materials().next().unwrap()).expect("material should convert");
		let graph = &material.graph;
		let surface = graph.surface();

		assert_eq!(material.name.as_deref(), Some("paint"));
		assert!(material.double_sided);
		assert_eq!(material.alpha_mode, AlphaMode::Mask(0.25));
		assert!(material.images.is_empty());
		assert_eq!(constant(graph, surface.metalness), Some(Value::Float(0.4)));
		assert_eq!(constant(graph, surface.roughness), Some(Value::Float(0.6)));
		assert_eq!(constant(graph, surface.emission), Some(Value::Vector3([0.1, 0.2, 0.3])));
		assert!(matches!(
			graph.node(surface.opacity.unwrap()),
			Node::Extract { value, index: 3 } if constant(graph, Some(*value)) == Some(Value::Vector4([0.25, 0.5, 0.75, 0.8]))
		));
	}

	#[test]
	fn texture_factors_multiply_their_textures_in_first_use_slots() {
		let gltf = parse_gltf(gltf_with_textures(
			r#"{
				"pbrMetallicRoughness":{
					"baseColorFactor":[0.2,0.3,0.4,0.5],
					"baseColorTexture":{"index":2},
					"metallicFactor":0.7,
					"roughnessFactor":0.8,
					"metallicRoughnessTexture":{"index":0}
				},
				"emissiveFactor":[0.4,0.5,0.6],
				"emissiveTexture":{"index":2}
			}"#,
		));

		let material = material_from_gltf(&gltf.materials().next().unwrap()).expect("material should convert");
		let graph = &material.graph;
		let surface = graph.surface();

		// Base colour samples image 2 first, so it takes slot zero, which the emission texture shares.
		assert_eq!(material.images, vec![2, 0]);

		let (factor, texture) = multiply(graph, surface.metalness.unwrap());
		assert_eq!(constant(graph, Some(factor)), Some(Value::Float(0.7)));
		assert!(matches!(
			graph.node(texture),
			Node::Extract { value, index: METALNESS_CHANNEL } if image(graph, *value) == Some((1, Type::Vector4))
		));

		let (factor, texture) = multiply(graph, surface.roughness.unwrap());
		assert_eq!(constant(graph, Some(factor)), Some(Value::Float(0.8)));
		assert!(matches!(
			graph.node(texture),
			Node::Extract {
				index: ROUGHNESS_CHANNEL,
				..
			}
		));

		let (factor, texture) = multiply(graph, surface.emission.unwrap());
		assert_eq!(constant(graph, Some(factor)), Some(Value::Vector3([0.4, 0.5, 0.6])));
		assert_eq!(image(graph, texture), Some((0, Type::Vector3)));
	}

	#[test]
	fn converts_gltf_normal_occlusion_and_blend_properties() {
		let gltf = parse_gltf(gltf_with_textures(
			r#"{
				"alphaMode":"BLEND",
				"normalTexture":{"index":0,"scale":0.5},
				"occlusionTexture":{"index":1,"strength":0.75}
			}"#,
		));

		let material = material_from_gltf(&gltf.materials().next().unwrap()).expect("material should convert");
		let graph = &material.graph;
		let surface = graph.surface();

		assert_eq!(material.alpha_mode, AlphaMode::Blend);

		let Node::NormalMap { value, scale } = graph.node(surface.normal.unwrap()) else {
			panic!("A normal texture should become a normal map.");
		};
		assert_eq!(image(graph, *value), Some((0, Type::Vector3)));
		assert_eq!(constant(graph, Some(*scale)), Some(Value::Float(0.5)));

		let Node::Mix {
			background,
			foreground,
			factor,
		} = graph.node(surface.occlusion.unwrap())
		else {
			panic!("Partial occlusion strength should blend toward no occlusion.");
		};
		assert_eq!(constant(graph, Some(*background)), Some(Value::Float(1.0)));
		assert_eq!(constant(graph, Some(*factor)), Some(Value::Float(0.75)));
		assert!(matches!(graph.node(*foreground), Node::Extract { index: 0, .. }));
	}

	fn constant(graph: &Graph, node: Option<NodeId>) -> Option<Value> {
		match graph.node(node?) {
			Node::Constant(value) => Some(*value),
			_ => None,
		}
	}

	fn multiply(graph: &Graph, node: NodeId) -> (NodeId, NodeId) {
		match graph.node(node) {
			Node::Binary {
				operator: BinaryOperator::Multiply,
				left,
				right,
			} => (*left, *right),
			other => panic!("Expected a multiply node, got {other:#?}"),
		}
	}

	/// Returns the slot and type of an image node.
	fn image(graph: &Graph, node: NodeId) -> Option<(usize, Type)> {
		match graph.node(node) {
			Node::Image { texture, r#type, .. } => material_texture_slot(texture).map(|slot| (slot, *r#type)),
			_ => None,
		}
	}

	fn parse_gltf(json: impl AsRef<[u8]>) -> gltf::Gltf {
		gltf::Gltf::from_slice(json.as_ref()).expect("test glTF should parse")
	}

	fn gltf_with_textures(material: &str) -> String {
		format!(
			r#"{{
				"asset":{{"version":"2.0"}},
				"images":[{{"uri":"a.png"}},{{"uri":"b.png"}},{{"uri":"c.png"}}],
				"textures":[{{"source":0}},{{"source":1}},{{"source":2}}],
				"materials":[{material}]
			}}"#,
		)
	}
}
