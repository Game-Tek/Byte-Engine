use polodb_core::bson;
use serde::Deserialize;

use crate::{types::Material, GenericResourceResponse, GenericResourceSerialization, ResourceResponse, TypedResourceDocument};

use super::resource_handler::{ReadTargets, ResourceHandler, ResourceReader};

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
		&["Material", "Shader", "Variant"]
	}

	fn read<'s, 'a>(&'s self, mut resource: GenericResourceResponse<'a>, mut reader: Box<dyn ResourceReader>,) -> utils::BoxedFuture<'a, Option<ResourceResponse<'a>>> {
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
	}
}