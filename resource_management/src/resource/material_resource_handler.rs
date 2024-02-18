use serde::Deserialize;
use smol::{fs::File, io::AsyncReadExt};

use crate::{types::{Material, Shader, ShaderTypes, Variant}, GenericResourceSerialization, ProcessedResources, Resource, Stream};

use super::{resource_handler::ResourceHandler, resource_manager::ResourceManager,};

pub struct MaterialResourcerHandler {}

pub trait ProgramGenerator: Sync + Send {
	/// Transforms a program.
	fn transform(&self, children: Vec<std::rc::Rc<jspd::Node>>) -> (&'static str, jspd::Node);
}

impl MaterialResourcerHandler {
	pub fn new() -> Self {
		Self {
		}
	}
}

impl ResourceHandler for MaterialResourcerHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool {
		match resource_type {
			"json" => true,
			"glsl" => true,
			"besl" => true,
			_ => false
		}
	}

	fn read<'a>(&'a self, _resource: &'a dyn Resource, file: &'a mut File, buffers: &'a mut [Stream<'a>]) -> utils::BoxedFuture<()> {
		Box::pin(async move { file.read_exact(buffers[0].buffer).await.unwrap(); })
	}

	fn get_deserializers(&self) -> Vec<(&'static str, Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn Resource> + Send>)> {
		vec![("Material",
			Box::new(|_document| {
				Box::new(Material::deserialize(polodb_core::bson::Deserializer::new(_document.into())).unwrap())
			})),
			("Shader",
			Box::new(|_document| {
				Box::new(Shader {
					stage: ShaderTypes::Compute,
				})
			})),
			("Variant",
			Box::new(|document| {
				Box::new(Variant::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap())
			})),
		]
	}
}

#[cfg(test)]
mod tests {
    use crate::resource::resource_manager::ResourceManager;

	#[test]
	#[ignore] // We need to implement a shader generator to test this
	fn load_material() {
		let mut resource_manager = ResourceManager::new();

		resource_manager.add_resource_handler(super::MaterialResourcerHandler::new());

		let (response, _) = smol::block_on(resource_manager.get("solid")).expect("Failed to load material");

		assert_eq!(response.resources.len(), 2); // 1 material, 1 shader

		let resource_container = &response.resources[0];

		assert_eq!(resource_container.class, "Shader");

		let resource_container = &response.resources[1];

		assert_eq!(resource_container.class, "Material");
	}
}