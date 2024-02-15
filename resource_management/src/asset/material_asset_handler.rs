use crate::{types::{AlphaMode, Material, Model, Property, Value, Variant, VariantVariable}, GenericResourceSerialization, ProcessedResources};

use super::{asset_handler::AssetHandler, read_asset_from_source};

struct MaterialAssetHandler {
}

impl MaterialAssetHandler {
	fn new() -> MaterialAssetHandler {
		MaterialAssetHandler {}
	}
}

impl AssetHandler for MaterialAssetHandler {
	fn load(&self, url: &str, json: &json::JsonValue) -> utils::BoxedFuture<Option<Result<(), String>>> {
		async move {
			let (data, at) = read_asset_from_source(url, None).await.unwrap();

			let asset_json = json::parse(std::str::from_utf8(&data).unwrap()).unwrap(); // False

			let is_material = asset_json["parent"].is_null();

			if is_material {
				let material_domain = match &asset_json["domain"] {
					json::JsonValue::Null => { "Common".to_string() }
					json::JsonValue::Short(s) => { s.to_string() }
					json::JsonValue::String(s) => { s.to_string() }
					_ => { panic!("Invalid domain") }
				};

				let _material_type = match &asset_json["type"] {
					json::JsonValue::Null => { "Raw".to_string() }
					json::JsonValue::Short(s) => { s.to_string() }
					json::JsonValue::String(s) => { s.to_string() }
					_ => { panic!("Invalid type") }
				};
				
				let mut required_resources = asset_json["shaders"].entries().filter_map(|(s_type, shader_json)| {
					// smol::block_on(self.produce_shader(resource_manager, &material_domain, &asset_json, &shader_json, s_type))
				}).collect::<Vec<_>>();

				for variable in asset_json["variables"].members() {
					if variable["data_type"].as_str().unwrap() == "Texture2D" {
						let texture_url = variable["value"].as_str().unwrap();

						required_resources.push(ProcessedResources::Reference(texture_url.to_string()));
					}
				}

				Ok(vec![ProcessedResources::Generated((GenericResourceSerialization::new(url.to_string(), Material {
					albedo: Property::Factor(Value::Vector3([1f32, 0f32, 0f32])),
					normal: Property::Factor(Value::Vector3([0f32, 0f32, 1f32])),
					roughness: Property::Factor(Value::Scalar(0.5f32)),
					metallic: Property::Factor(Value::Scalar(0.0f32)),
					emissive: Property::Factor(Value::Vector3([0f32, 0f32, 0f32])),
					occlusion: Property::Factor(Value::Scalar(0f32)),
					double_sided: false,
					alpha_mode: AlphaMode::Opaque,
					model: Model {
						name: Self::RENDER_MODEL.to_string(),
						pass: "MaterialEvaluation".to_string(),
					},
				}).required_resources(&required_resources), Vec::new()))])
			} else {
				let variant_json = asset_json;

				let parent_material_url = variant_json["parent"].as_str().unwrap();

				let material_resource_document = GenericResourceSerialization::new(url.to_string(), Variant{
					parent: parent_material_url.to_string(),
					variables: variant_json["variables"].members().map(|v| {
						VariantVariable {
							name: v["name"].to_string(),
							value: v["value"].to_string(),
						}
					}).collect::<Vec<_>>()
				}).required_resources(&[ProcessedResources::Reference(parent_material_url.to_string())]);

				Ok(vec![ProcessedResources::Generated((material_resource_document.into(), Vec::new()))])
			}

			Ok(())
		}.boxed()
	}
}