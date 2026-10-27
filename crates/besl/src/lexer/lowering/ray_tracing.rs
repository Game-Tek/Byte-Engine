//! Links the ray tracing parts of a program that depend on the program's own types: the payload that a closest-hit
//! `main` takes and returns, and the `invoke` calls of a ray generation shader, whose signature comes from the payload
//! they pass.

use super::*;

const RAY_TRACING_ENTRY_DOCS_PATH: &str = "reference/besl/interface#ray-tracing-entry-points";
const RAY_GENERATION_DOCS_PATH: &str = "reference/besl/intrinsics#ray-generation-stage";
const DESCRIPTORS_DOCS_PATH: &str = "reference/besl/interface#descriptors";

/// The name of the call that runs the closest hit of a hit's record.
pub(super) const INVOKE_INTRINSIC: &str = "invoke";

/// The member types a payload can carry: scalars and vectors of `u16`, `u32`, `i32`, `f16`, and `f32`, which every
/// backend passes between functions with one layout.
const PAYLOAD_MEMBER_TYPES: [&str; 17] = [
	"u16", "u32", "i32", "f16", "f32", "vec2u16", "vec4u16", "vec2u", "vec3u", "vec4u", "vec2i", "vec2f16", "vec3f16",
	"vec4f16", "vec2f", "vec3f", "vec4f",
];

/// Rejects a type that can't be a ray tracing payload: anything but a declared struct with at least one member, whose
/// members are all payload member types.
pub(in crate::lexer) fn validate_payload(r#type: &NodeReference) -> Result<(), LexError> {
	let r#type = r#type.borrow();
	let docs = crate::online_docs_url(RAY_TRACING_ENTRY_DOCS_PATH);
	let name = r#type.get_name().unwrap_or("this value");
	let fields = match r#type.node() {
		Nodes::Struct {
			template: None, fields, ..
		} if !fields.is_empty() && builtin_type(name).is_none() => fields,
		_ => {
			return Err(LexError::invalid(format!(
				"`{name}` can't be a ray tracing payload. The most likely cause is that it isn't a struct you declared, or that the struct has no members. Declare a struct whose members are scalars or vectors of `u16`, `u32`, `i32`, `f16`, or `f32`, and use it as the payload. See {docs}."
			)));
		}
	};
	for field in fields {
		let field = field.borrow();
		let Nodes::Member {
			name: member,
			r#type: member_type,
			count,
		} = field.node()
		else {
			continue;
		};
		let member_type = member_type.borrow();
		let member_type = member_type.get_name().unwrap_or("unknown");
		if count.is_some() || !PAYLOAD_MEMBER_TYPES.contains(&member_type) {
			let member_type = count.map_or_else(|| member_type.to_string(), |count| format!("{member_type}[{count}]"));
			return Err(LexError::invalid(format!(
				"Payload `{name}` can't carry member `{member}` of type `{member_type}`. The most likely cause is a member that is a `bool`, a `u8`, an array, a matrix, an atomic, or a struct, which payloads can't hold. Change `{member}` to a scalar or vector of `u16`, `u32`, `i32`, `f16`, or `f32`. See {docs}."
			)));
		}
	}
	Ok(())
}

/// Rejects a closest-hit `main` whose payload can't be one. Entry normalization leaves a closest hit's payload as the
/// only parameter of a `main` that returns the payload's type.
pub(super) fn validate_entry_payload(function: &NodeReference) -> Result<(), LexError> {
	let function = function.borrow();
	let Nodes::Function {
		name,
		params,
		return_type,
		..
	} = function.node()
	else {
		return Ok(());
	};
	let [parameter] = params.as_slice() else {
		return Ok(());
	};
	if name != "main" {
		return Ok(());
	}
	match parameter.borrow().node() {
		Nodes::Parameter { r#type, .. } if r#type == return_type => validate_payload(r#type),
		_ => Ok(()),
	}
}

/// Rejects an `AccelerationStructure` descriptor that declares write access or a count. Shaders only read a scene, and
/// one descriptor binds one structure.
pub(super) fn validate_acceleration_structure_descriptor(
	name: &str,
	write: bool,
	count: Option<std::num::NonZeroU32>,
) -> Result<(), LexError> {
	let docs = crate::online_docs_url(DESCRIPTORS_DOCS_PATH);
	if write {
		return Err(LexError::invalid(format!(
			"Acceleration structure descriptor `{name}` declares write access. The most likely cause is `access: write` or `access: read_write` on an `AccelerationStructure` descriptor, which shaders only read. Set `access: read`. See {docs}."
		)));
	}
	if count.is_some() {
		return Err(LexError::invalid(format!(
			"Acceleration structure descriptor `{name}` declares a count. The most likely cause is `count` on an `AccelerationStructure` descriptor, which binds one structure. Remove `count`, and declare one descriptor for each structure. See {docs}."
		)));
	}
	Ok(())
}

impl Lexer {
	/// Links an `invoke(hit, payload)` call. Its signature depends on the payload type, so the first call instantiates
	/// the intrinsic for its payload, and every later call in the program must pass the same type and share it.
	pub(super) fn lex_invoke(&mut self, arguments: Vec<NodeReference>) -> Result<Node, LexError> {
		let docs = crate::online_docs_url(RAY_GENERATION_DOCS_PATH);
		let [hit, payload] = arguments.as_slice() else {
			return Err(LexError::invalid(format!(
				"`invoke` takes 2 arguments, but this call passes {count}. The most likely cause is a missing or extra argument. Pass the hit and the payload, as in `invoke(hit, payload)`. See {docs}.",
				count = arguments.len()
			)));
		};
		let hit_type = infer_expression_type(hit).filter(|r#type| r#type.borrow().get_name() == Some(crate::HIT_STRUCT));
		let Some(hit_type) = hit_type else {
			return Err(LexError::invalid(format!(
				"`invoke` takes a `Hit` as its first argument, but this call passes another value. The most likely cause is that the arguments are swapped, or that the first one isn't a hit from `intersect` or the `Hit` constructor. Pass the hit first and the payload second. See {docs}."
			)));
		};
		let Some(payload_type) = infer_expression_type(payload) else {
			return Err(LexError::invalid(format!(
				"`invoke` can't tell the payload type of its second argument. The most likely cause is an argument that isn't a payload value, such as a descriptor. Pass a local or a constructed value of the payload struct. See {docs}."
			)));
		};
		validate_payload(&payload_type)?;

		let intrinsic = match &self.invoke {
			Some(intrinsic) => {
				let expected =
					infer_callable_return_type(intrinsic).expect("`invoke` is an intrinsic, which has a return type");
				if expected != payload_type {
					let expected = expected.borrow();
					let payload_type = payload_type.borrow();
					return Err(LexError::invalid(format!(
						"`invoke` passes payload `{found}`, but an earlier `invoke` in this shader passes `{expected}`. The most likely cause is a shader that uses two payload structs, which one pipeline can't have. Pass a `{expected}` to every `invoke` call. See {docs}.",
						found = payload_type.get_name().unwrap_or("unknown"),
						expected = expected.get_name().unwrap_or("unknown"),
					)));
				}
				intrinsic.clone()
			}
			None => {
				let intrinsic = super::super::ast::builtin_intrinsic(
					INVOKE_INTRINSIC,
					vec![("hit", hit_type), ("payload", payload_type.clone())],
					payload_type,
				);
				self.invoke = Some(intrinsic.clone());
				intrinsic
			}
		};

		// A body-less intrinsic expands to its arguments, as `build_intrinsic` expands the registered ones.
		Ok(Node::expression(Expressions::IntrinsicCall {
			intrinsic,
			elements: arguments.clone(),
			arguments,
		}))
	}
}
