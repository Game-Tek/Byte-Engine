pub trait ShaderGenerator {
	fn process(&self) -> (&'static str, json::JsonValue);
}