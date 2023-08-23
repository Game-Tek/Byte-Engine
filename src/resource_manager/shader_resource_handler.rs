use serde::{Serialize, Deserialize};

use crate::render_backend;

use super::ResourceHandler;

#[derive(Debug, Serialize, Deserialize)]
pub struct Shader {
	pub stage: render_backend::ShaderTypes,
}

pub struct ShaderResourceHandler {

}

impl ShaderResourceHandler {
	pub fn new() -> Self {
		Self {

		}
	}
}

impl ResourceHandler for ShaderResourceHandler {
	fn can_handle_type(&self, resource_type: &str) -> bool {
		match resource_type {
			_ => false
		}
	}

	fn process(&self, bytes: &[u8]) -> Result<Vec<(polodb_core::bson::Document, Vec<u8>)>, String> {
		Ok(vec![(polodb_core::bson::Document::new(), bytes.to_vec())])
	}

	fn get_deserializer(&self) -> Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send> {
		Box::new(|document| {
			Box::new(Shader::deserialize(polodb_core::bson::Deserializer::new(document.into())).unwrap())
		})
	}
}