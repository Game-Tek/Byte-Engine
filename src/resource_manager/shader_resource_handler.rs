use super::ResourceHandler;

pub struct Shader {

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
			"json" => true,
			_ => false
		}
	}

	fn process(&self, bytes: Vec<u8>) -> Result<Vec<(polodb_core::bson::Document, Vec<u8>)>, String> {
		Err("Not implemented".to_string())
	}

	fn get_deserializer(&self) -> Box<dyn Fn(&polodb_core::bson::Document) -> Box<dyn std::any::Any> + Send> {
		Box::new(|document| {
			Box::new(Shader {})
		})
	}
}