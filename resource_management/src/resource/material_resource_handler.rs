use polodb_core::bson;
use serde::Deserialize;
use smol::{fs::File, io::AsyncReadExt};

use crate::{types::{Material, Shader, ShaderTypes, Variant}, GenericResourceSerialization, ProcessedResources, Resource, ResourceResponse, Stream};

use super::{resource_handler::{ReadTargets, ResourceHandler, ResourceReader}, resource_manager::ResourceManager,};

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
	fn get_handled_resource_classes<'a>(&self,) -> &'a [&'a str] {
		&["Material", "Shader", "Variant"]
	}

	fn read<'a>(&'a self, resource: &'a GenericResourceSerialization, file: &'a mut dyn ResourceReader, _: &'a mut ReadTargets<'a>) -> utils::BoxedFuture<'a, Option<ResourceResponse>> {
		// vec![("Material",
		// 	Box::new(|_document| {
		// 		Box::new(Material::deserialize(polodb_core::bson::Deserializer::new(_document.into())).unwrap())
		// 	})),
		// 	("Shader",
		// 	Box::new(|_document| {
		// 		Box::new(Shader {
		// 			stage: ShaderTypes::Compute,
		// 		})
		// 	})),
		// 	("Variant",
		// 	Box::new(|document| {
		// 		Box::new(Variant::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap())
		// 	})),
		// ]

		Box::pin(async move {
			let material_resource = Material::deserialize(bson::Deserializer::new(resource.resource.clone().into())).ok()?;
			Some(ResourceResponse::new(resource, material_resource))
		})
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