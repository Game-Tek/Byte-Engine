pub mod glsl;
pub mod hlsl;
pub mod msl;
pub mod platform;
pub mod spirv;

/// Returns whether a linked expression is the scalar value two, optionally wrapped in a scalar cast.
fn is_two(node: &besl::NodeReference) -> bool {
	match node.borrow().node() {
		besl::Nodes::Expression(besl::Expressions::Literal { value }) => value.parse::<f64>() == Ok(2.0),
		besl::Nodes::Expression(besl::Expressions::IntrinsicCall {
			intrinsic, arguments, ..
		}) if arguments.len() == 1 && matches!(intrinsic.borrow().get_name(), Some("f16" | "f32")) => is_two(&arguments[0]),
		_ => false,
	}
}
