use polodb_core::bson;
use serde::Deserialize;

use crate::{types::{Material, Variant}, GenericResourceResponse, ResourceResponse};

use super::resource_handler::{ResourceHandler, ResourceReader};

pub struct MaterialResourcerHandler {}

pub trait ProgramGenerator: Sync + Send {
	/// Transforms a program.
	fn transform(&self, scope: jspd::NodeReference) -> jspd::NodeReference;
}

impl MaterialResourcerHandler {
	pub fn new() -> Self {
		Self {
		}
	}
}

impl ResourceHandler for MaterialResourcerHandler {
	fn get_handled_resource_classes<'a>(&self,) -> &'a [&'a str] {
		&["Material", "Variant"]
	}

	fn read<'s, 'a>(&'s self, meta_resource: GenericResourceResponse<'a>, reader: Option<Box<dyn ResourceReader>>,) -> utils::BoxedFuture<'a, Option<ResourceResponse<'a>>> {
		Box::pin(async move {
			match meta_resource.class.as_str() {
				"Material" => {
					let resource = Material::deserialize(bson::Deserializer::new(meta_resource.resource.clone().into())).ok()?;
					Some(ResourceResponse::new(meta_resource, resource))
				}
				"Variant" => {
					let resource = Variant::deserialize(bson::Deserializer::new(meta_resource.resource.clone().into())).ok()?;
					Some(ResourceResponse::new(meta_resource, resource))
				}
				_ => {
					return None;
				}
			}
		})
	}
}

#[cfg(test)]
mod tests {
    use crate::{asset::{asset_handler::AssetHandler, material_asset_handler::{tests::TestShaderGenerator, MaterialAssetHandler}, tests::{TestAssetResolver, TestStorageBackend}}, resource::{material_resource_handler::MaterialResourcerHandler, resource_handler::ResourceHandler}, types::{AlphaMode, Material}, StorageBackend};

	#[test]
	fn load_material() {
		// Create resource from asset

		let mut asset_handler = MaterialAssetHandler::new();

		let url = "material.json";
		let doc = json::object! {
			"url": url,
		};

		let asset_resolver = TestAssetResolver::new();
		let storage_backend = TestStorageBackend::new();

		let shader_generator = TestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		let url = "material.json";

		let material_json = r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Fragment": "fragment.besl"
			},
			"variables": [
				{
					"name": "color",
					"data_type": "vec4f",
					"type": "Static",
					"value": "Purple"
				}
			]
		}"#;

		asset_resolver.add_file(url, material_json.as_bytes());

		let shader_file = "main: fn () -> void {
			material;
		}";

		asset_resolver.add_file("fragment.besl", shader_file.as_bytes());

		let doc = json::object! {
			"url": url,
		};

		smol::block_on(asset_handler.load(&asset_resolver, &storage_backend, url, &doc)).expect("Material asset handler did not handle asset").expect("Material asset handler failed to load asset");

		// Load resource from storage

		let material_resource_handler = MaterialResourcerHandler::new();

		let (resource, reader) = smol::block_on(storage_backend.read(url)).expect("Failed to read asset from storage");

		let resource = smol::block_on(material_resource_handler.read(resource, Some(reader),)).unwrap();

		assert_eq!(resource.id(), "material.json");
		assert_eq!(resource.class, "Material");

		let material = resource.resource.downcast_ref::<Material>().unwrap();

		assert_eq!(material.double_sided, false);
		assert_eq!(material.alpha_mode, AlphaMode::Opaque);
	}
}