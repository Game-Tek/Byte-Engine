use crate::{types::{Material, MaterialModel, Variant, VariantModel}, GenericResourceResponse, ResourceResponse, StorageBackend, TypedResourceModel, Solver};

use super::resource_handler::{ResourceHandler, ResourceReader};

pub struct MaterialResourcerHandler {}

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

	fn read<'s, 'a, 'b>(&'s self, meta_resource: GenericResourceResponse<'a>, _: Option<Box<dyn ResourceReader>>, storage_backend: &'b dyn StorageBackend) -> utils::BoxedFuture<'b, Option<ResourceResponse<'a>>> where 'a: 'b {
		Box::pin(async move {
			match meta_resource.class.as_str() {
				"Material" => {
					let resource: TypedResourceModel<MaterialModel> = meta_resource.into();
					let material = resource.solve(storage_backend).ok()?;
					Some(material.into())
				}
				"Variant" => {
					let resource: TypedResourceModel<VariantModel> = meta_resource.into();
					let variant = resource.solve(storage_backend).ok()?;
					Some(variant.into())
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
    use crate::{asset::{asset_handler::AssetHandler, material_asset_handler::{tests::RootTestShaderGenerator, MaterialAssetHandler}, tests::{TestAssetResolver, TestStorageBackend}}, resource::{material_resource_handler::MaterialResourcerHandler, resource_handler::ResourceHandler}, types::{AlphaMode, Material, ShaderTypes}, StorageBackend};

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

		let shader_generator = RootTestShaderGenerator::new();

		asset_handler.set_shader_generator(shader_generator);

		let url = "material.json";

		let material_json = r#"{
			"domain": "World",
			"type": "Surface",
			"shaders": {
				"Compute": "fragment.besl"
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
		}";

		asset_resolver.add_file("fragment.besl", shader_file.as_bytes());

		let doc = json::object! {
			"url": url,
		};

		smol::block_on(asset_handler.load(&asset_resolver, &storage_backend, url, &doc)).expect("Material asset handler did not handle asset").expect("Material asset handler failed to load asset");

		// Load resource from storage

		let material_resource_handler = MaterialResourcerHandler::new();

		let (resource, reader) = smol::block_on(storage_backend.read(url)).expect("Failed to read asset from storage");

		let resource = smol::block_on(material_resource_handler.read(resource, Some(reader), &storage_backend)).unwrap();

		assert_eq!(resource.id(), "material.json");
		assert_eq!(resource.class, "Material");

		let material = resource.resource.downcast_ref::<Material>().unwrap();

		assert_eq!(material.double_sided, false);
		assert_eq!(material.alpha_mode, AlphaMode::Opaque);
		assert_eq!(material.shaders().len(), 1);

		let shader = material.shaders().get(0).unwrap();

		assert_eq!(shader.resource.stage, ShaderTypes::Compute);
		assert!(shader.get_buffer().is_some());
		assert!(shader.get_buffer().unwrap().len() > 0);
	}
}