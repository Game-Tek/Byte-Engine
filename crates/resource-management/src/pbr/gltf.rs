use super::{
	BrdfAlphaMode, BrdfChannel, BrdfMaterialBuilder, BrdfMaterialDescription, BrdfMetallicRoughness, BrdfNode, BrdfNodeId,
	BrdfTexture, BrdfValue,
};

/// Converts a glTF material into a flat BRDF material graph.
pub fn brdf_material_from_gltf(material: &gltf::Material) -> BrdfMaterialDescription {
	let pbr = material.pbr_metallic_roughness();
	let mut builder = BrdfMaterialBuilder::new();

	let base_color = factor_and_optional_texture(
		&mut builder,
		BrdfValue::Vector4(pbr.base_color_factor()),
		pbr.base_color_texture()
			.map(|texture| texture_info(texture.texture(), texture.tex_coord())),
	);

	let metallic_roughness_texture = pbr
		.metallic_roughness_texture()
		.map(|texture| builder.texture(texture_info(texture.texture(), texture.tex_coord())));

	let metallic_factor = builder.constant(BrdfValue::Scalar(pbr.metallic_factor()));
	let metallic = if let Some(texture) = metallic_roughness_texture {
		let metallic_channel = builder.extract_channel(texture, BrdfChannel::Blue);
		builder.multiply(metallic_factor, metallic_channel)
	} else {
		metallic_factor
	};

	let roughness_factor = builder.constant(BrdfValue::Scalar(pbr.roughness_factor()));
	let roughness = if let Some(texture) = metallic_roughness_texture {
		let roughness_channel = builder.extract_channel(texture, BrdfChannel::Green);
		builder.multiply(roughness_factor, roughness_channel)
	} else {
		roughness_factor
	};

	let normal = material.normal_texture().map(|texture| {
		let source = builder.texture(texture_info(texture.texture(), texture.tex_coord()));
		builder.add(BrdfNode::NormalMap {
			source,
			scale: texture.scale(),
		})
	});

	let occlusion = material.occlusion_texture().map(|texture| {
		let source = builder.texture(texture_info(texture.texture(), texture.tex_coord()));
		builder.add(BrdfNode::Occlusion {
			source,
			strength: texture.strength(),
		})
	});

	let emission = factor_and_optional_texture(
		&mut builder,
		BrdfValue::Vector3(material.emissive_factor()),
		material
			.emissive_texture()
			.map(|texture| texture_info(texture.texture(), texture.tex_coord())),
	);
	let emission = builder.add(BrdfNode::Emission { color: emission });

	let surface = builder.add(BrdfNode::MetallicRoughness(BrdfMetallicRoughness {
		base_color,
		metallic,
		roughness,
		normal,
		occlusion,
		emission: Some(emission),
	}));

	builder.finish(
		material.name().map(ToString::to_string),
		surface,
		material.double_sided(),
		brdf_alpha_mode_from_gltf(material),
	)
}

fn factor_and_optional_texture(
	builder: &mut BrdfMaterialBuilder,
	factor: BrdfValue,
	texture: Option<BrdfTexture>,
) -> BrdfNodeId {
	let factor = builder.constant(factor);
	if let Some(texture) = texture {
		let texture = builder.texture(texture);
		builder.multiply(factor, texture)
	} else {
		factor
	}
}

fn texture_info(texture: gltf::Texture, texcoord_channel: u32) -> BrdfTexture {
	BrdfTexture {
		image_index: texture.source().index() as u32,
		texcoord_channel,
	}
}

fn brdf_alpha_mode_from_gltf(material: &gltf::Material) -> BrdfAlphaMode {
	match material.alpha_mode() {
		gltf::material::AlphaMode::Opaque => BrdfAlphaMode::Opaque,
		gltf::material::AlphaMode::Mask => BrdfAlphaMode::Mask(material.alpha_cutoff().unwrap_or(0.5)),
		gltf::material::AlphaMode::Blend => BrdfAlphaMode::Blend,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn converts_default_gltf_material_to_valid_default_brdf_graph() {
		let gltf = parse_gltf(r#"{"asset":{"version":"2.0"},"materials":[{}]}"#);
		let material = gltf.materials().next().unwrap();

		let brdf = brdf_material_from_gltf(&material);

		assert_eq!(brdf.name, None);
		assert_eq!(brdf.double_sided, false);
		assert_eq!(brdf.alpha_mode, BrdfAlphaMode::Opaque);
		assert_eq!(brdf.validate(), Ok(()));
		let surface = expect_surface(&brdf);
		assert_eq!(
			brdf.nodes[surface.base_color.index()],
			BrdfNode::Constant(BrdfValue::Vector4([1.0, 1.0, 1.0, 1.0]))
		);
		assert_eq!(
			brdf.nodes[surface.metallic.index()],
			BrdfNode::Constant(BrdfValue::Scalar(1.0))
		);
		assert_eq!(
			brdf.nodes[surface.roughness.index()],
			BrdfNode::Constant(BrdfValue::Scalar(1.0))
		);
		assert!(surface.normal.is_none());
		assert!(surface.occlusion.is_none());
		assert!(surface.emission.is_some());
	}

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
		let material = gltf.materials().next().unwrap();

		let brdf = brdf_material_from_gltf(&material);

		assert_eq!(brdf.name.as_deref(), Some("paint"));
		assert_eq!(brdf.double_sided, true);
		assert_eq!(brdf.alpha_mode, BrdfAlphaMode::Mask(0.25));
		assert_eq!(brdf.validate(), Ok(()));
		let surface = expect_surface(&brdf);
		assert_eq!(
			brdf.nodes[surface.base_color.index()],
			BrdfNode::Constant(BrdfValue::Vector4([0.25, 0.5, 0.75, 0.8]))
		);
		assert_eq!(
			brdf.nodes[surface.metallic.index()],
			BrdfNode::Constant(BrdfValue::Scalar(0.4))
		);
		assert_eq!(
			brdf.nodes[surface.roughness.index()],
			BrdfNode::Constant(BrdfValue::Scalar(0.6))
		);
	}

	#[test]
	fn converts_gltf_texture_factors_to_multiply_nodes() {
		let gltf = parse_gltf(gltf_with_textures(
			r#"{
				"pbrMetallicRoughness":{
					"baseColorFactor":[0.2,0.3,0.4,0.5],
					"baseColorTexture":{"index":0,"texCoord":1},
					"metallicFactor":0.7,
					"roughnessFactor":0.8,
					"metallicRoughnessTexture":{"index":1,"texCoord":2}
				},
				"emissiveFactor":[0.4,0.5,0.6],
				"emissiveTexture":{"index":2,"texCoord":3}
			}"#,
		));
		let material = gltf.materials().next().unwrap();

		let brdf = brdf_material_from_gltf(&material);

		assert_eq!(brdf.validate(), Ok(()));
		let surface = expect_surface(&brdf);
		let base_color_multiply = expect_multiply(&brdf, surface.base_color);
		assert_eq!(
			brdf.nodes[base_color_multiply.left.index()],
			BrdfNode::Constant(BrdfValue::Vector4([0.2, 0.3, 0.4, 0.5]))
		);
		assert_eq!(
			brdf.nodes[base_color_multiply.right.index()],
			BrdfNode::Texture(BrdfTexture {
				image_index: 0,
				texcoord_channel: 1,
			})
		);

		let metallic_multiply = expect_multiply(&brdf, surface.metallic);
		assert_eq!(
			brdf.nodes[metallic_multiply.left.index()],
			BrdfNode::Constant(BrdfValue::Scalar(0.7))
		);
		assert_eq!(
			brdf.nodes[metallic_multiply.right.index()],
			BrdfNode::ExtractChannel {
				source: BrdfNodeId::new(3),
				channel: BrdfChannel::Blue,
			}
		);
		assert_eq!(
			brdf.nodes[3],
			BrdfNode::Texture(BrdfTexture {
				image_index: 1,
				texcoord_channel: 2,
			})
		);

		let roughness_multiply = expect_multiply(&brdf, surface.roughness);
		assert_eq!(
			brdf.nodes[roughness_multiply.left.index()],
			BrdfNode::Constant(BrdfValue::Scalar(0.8))
		);
		assert_eq!(
			brdf.nodes[roughness_multiply.right.index()],
			BrdfNode::ExtractChannel {
				source: BrdfNodeId::new(3),
				channel: BrdfChannel::Green,
			}
		);

		let emission = expect_emission(&brdf, surface.emission.unwrap());
		assert!(matches!(brdf.nodes[emission.color.index()], BrdfNode::Multiply { .. }));
	}

	#[test]
	fn converts_gltf_normal_occlusion_and_blend_properties() {
		let gltf = parse_gltf(gltf_with_textures(
			r#"{
				"alphaMode":"BLEND",
				"normalTexture":{"index":0,"texCoord":1,"scale":0.5},
				"occlusionTexture":{"index":1,"texCoord":2,"strength":0.75}
			}"#,
		));
		let material = gltf.materials().next().unwrap();

		let brdf = brdf_material_from_gltf(&material);

		assert_eq!(brdf.alpha_mode, BrdfAlphaMode::Blend);
		assert_eq!(brdf.validate(), Ok(()));
		let surface = expect_surface(&brdf);
		let normal_id = surface.normal.unwrap();
		let occlusion_id = surface.occlusion.unwrap();
		assert_eq!(
			brdf.nodes[normal_id.index()],
			BrdfNode::NormalMap {
				source: BrdfNodeId::new(3),
				scale: 0.5,
			}
		);
		assert_eq!(
			brdf.nodes[3],
			BrdfNode::Texture(BrdfTexture {
				image_index: 0,
				texcoord_channel: 1,
			})
		);
		assert_eq!(
			brdf.nodes[occlusion_id.index()],
			BrdfNode::Occlusion {
				source: BrdfNodeId::new(5),
				strength: 0.75,
			}
		);
		assert_eq!(
			brdf.nodes[5],
			BrdfNode::Texture(BrdfTexture {
				image_index: 1,
				texcoord_channel: 2,
			})
		);
	}

	#[test]
	fn converts_mask_alpha_without_cutoff_to_gltf_default() {
		let gltf = parse_gltf(r#"{"asset":{"version":"2.0"},"materials":[{"alphaMode":"MASK"}]}"#);
		let material = gltf.materials().next().unwrap();

		let brdf = brdf_material_from_gltf(&material);

		assert_eq!(brdf.alpha_mode, BrdfAlphaMode::Mask(0.5));
	}

	fn expect_surface(material: &BrdfMaterialDescription) -> BrdfMetallicRoughness {
		match material.node(material.surface).unwrap() {
			BrdfNode::MetallicRoughness(surface) => *surface,
			other => panic!("Expected surface node, got {other:#?}"),
		}
	}

	struct MultiplyParts {
		left: BrdfNodeId,
		right: BrdfNodeId,
	}

	fn expect_multiply(material: &BrdfMaterialDescription, id: BrdfNodeId) -> MultiplyParts {
		match material.node(id).unwrap() {
			BrdfNode::Multiply { left, right } => MultiplyParts {
				left: *left,
				right: *right,
			},
			other => panic!("Expected multiply node, got {other:#?}"),
		}
	}

	struct EmissionParts {
		color: BrdfNodeId,
	}

	fn expect_emission(material: &BrdfMaterialDescription, id: BrdfNodeId) -> EmissionParts {
		match material.node(id).unwrap() {
			BrdfNode::Emission { color } => EmissionParts { color: *color },
			other => panic!("Expected emission node, got {other:#?}"),
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
