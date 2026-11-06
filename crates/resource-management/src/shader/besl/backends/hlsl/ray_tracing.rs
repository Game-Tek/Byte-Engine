//! Lowers ray generation and closest-hit shaders, and the ray queries compute and ray generation shaders run, to HLSL
//! for DXR.
//!
//! The lowering follows the contract the GLSL lowering and the Vulkan backend share, which the ray tracing design note
//! (`docs/develop/rendering/ray-tracing.mdx`) describes. A ray generation shader is a `raygeneration` shader that finds
//! hits with inline ray queries and runs closest hits with `CallShader`, and a closest hit is a `callable` shader. The
//! two exchange [`CALL_STRUCT`]: the payload and the hit it is shaded for. Resources need no plumbing, because every
//! shader of a DXR pipeline reads the descriptor heaps through the same global root signature.

use std::fmt::Write as _;

use super::super::{IntrinsicRequirements, hit_member};
use super::Generator;
use crate::shader::generator::{NodeEmitter, ShaderFormatting, Stages};

/// Names the struct a ray generation shader and the closest hits it runs exchange: the payload, then the hit.
pub(crate) const CALL_STRUCT: &str = "_besl_call";

/// Names a closest hit's copy of the call it shades, which its helpers read the hit from.
///
/// DXR passes a callable shader its data as an entry point parameter, which helpers can't reach, so the entry point
/// copies it into this static and back.
pub(crate) const CALL_DATA: &str = "_besl_call_data";

/// Names the function that runs a closest hit's `main`, which the callable entry point calls.
pub(crate) const CLOSEST_HIT_MAIN: &str = "_besl_closest_hit_main";

/// Names the form a payload takes inside [`CALL_STRUCT`]: the payload with each 16-bit member widened to 32 bits.
///
/// Known issue, worked around here: DXR on the NVIDIA RTX 3060 (DXC 1.9, shader model 6.9) hands a ray generation
/// shader back the call data it passed to `CallShader`, untouched or zeroed, whenever that data holds a 16-bit member,
/// aligned or not. Callable data of 32-bit members only round-trips, so the payload crosses the call widened and the
/// shaders on both sides convert it. Surfel GI showed it first: every hit read as a back face, so every surfel was
/// voted out as buried, and probe GI came out white.
///
/// The widening costs a conversion on each side of a call. To remove it once a driver or DXC update fixes the issue,
/// first check that a `float16_t3` member in the `Payload` of the HLSL test shaders in
/// `ghi/tests/rendering/ray_tracing.rs` round-trips through the hit group tests on DX12.
pub(crate) const CALL_PAYLOAD_STRUCT: &str = "_besl_call_payload";

/// Names the functions that convert a payload to [`CALL_PAYLOAD_STRUCT`] and back.
const WIDEN_PAYLOAD: &str = "_besl_widen_payload";
const NARROW_PAYLOAD: &str = "_besl_narrow_payload";

/// Returns the 32-bit HLSL type that carries a 16-bit BESL scalar or vector across `CallShader`, or `None` for a type
/// that crosses as it is.
fn widened_call_type(besl_type: &str) -> Option<&'static str> {
	match besl_type {
		"f16" => Some("float"),
		"vec2f16" => Some("float2"),
		"vec3f16" => Some("float3"),
		"vec4f16" => Some("float4"),
		"u16" => Some("uint32_t"),
		"vec2u16" => Some("uint2"),
		"vec3u16" => Some("uint3"),
		"vec4u16" => Some("uint4"),
		_ => None,
	}
}

/// Reports whether `type_node`, a member's type, is or holds a 16-bit value inside a struct.
fn holds_16_bit_values(type_node: &besl::NodeReference) -> bool {
	let node = type_node.borrow();
	match node.node() {
		besl::Nodes::Struct { name, fields, .. } => {
			widened_call_type(name).is_some()
				|| fields.iter().any(|field| match field.borrow().node() {
					besl::Nodes::Member { r#type, .. } => holds_16_bit_values(r#type),
					_ => false,
				})
		}
		_ => false,
	}
}

impl Generator {
	/// Reports whether the shader needs the ray tracing prelude: it is a ray tracing stage, runs ray queries, or
	/// declares a `Hit` or an acceleration structure.
	pub(crate) fn uses_ray_tracing(stage: Stages, order: &[besl::NodeReference], requirements: &IntrinsicRequirements) -> bool {
		stage.is_ray_tracing()
			|| requirements.uses_intersect
			|| requirements.uses_trace_occlusion
			|| order.iter().any(super::super::declares_ray_tracing)
	}

	/// Emits what a shader that uses ray tracing declares before its own code: the `Hit` struct and the helpers of the
	/// ray queries the shader runs.
	///
	/// The shader's own emission skips the `Hit` node, so the struct is declared once, before the helpers return it.
	pub(crate) fn emit_ray_tracing_prelude(&self, string: &mut String, requirements: &IntrinsicRequirements) {
		let hit = besl::HIT_STRUCT;
		let _ = write!(string, "struct {hit}{{");
		for (name, r#type) in besl::HIT_MEMBERS {
			Self::emit_type_name(string, r#type);
			string.push(' ');
			Self::identifier(name).push_to(string);
			string.push(';');
		}
		string.push_str("};\n");

		let member = |name: &str| Self::identifier(name).to_string();
		let fill_hit = |record: &str, distance: &str, custom: &str, geometry: &str, primitive: &str, barycentrics: &str| {
			format!(
				"{hit} hit;hit.{}={record};hit.{}={distance};hit.{}={custom};hit.{}={geometry};hit.{}={primitive};hit.{}={barycentrics};hit.{}=origin;hit.{}=direction;return hit;",
				member("record"),
				member("distance"),
				member("custom_index"),
				member("geometry_index"),
				member("primitive_index"),
				member("barycentrics"),
				member("origin"),
				member("direction"),
			)
		};
		if requirements.uses_intersect {
			// Every triangle is opaque, so the query commits the nearest triangle without asking the shader, and the loop
			// that would confirm candidates runs no iteration. A hit's record is its instance's hit group offset plus its
			// geometry's index, which is what every backend computes.
			let miss = fill_hit("0", "-1.0", "0", "0", "0", "float2(0.0,0.0)");
			let found = fill_hit(
				"query.CommittedInstanceContributionToHitGroupIndex()+query.CommittedGeometryIndex()",
				"query.CommittedRayT()",
				"query.CommittedInstanceID()",
				"query.CommittedGeometryIndex()",
				"query.CommittedPrimitiveIndex()",
				"query.CommittedTriangleBarycentrics()",
			);
			let _ = write!(
				string,
				"{hit} _besl_intersect(RaytracingAccelerationStructure scene,uint mask,float3 origin,float3 direction,float t_min,float t_max){{\n\
				 \tRayQuery<RAY_FLAG_FORCE_OPAQUE> query;\n\
				 \tRayDesc ray;ray.Origin=origin;ray.Direction=direction;ray.TMin=t_min;ray.TMax=t_max;\n\
				 \tquery.TraceRayInline(scene,RAY_FLAG_NONE,mask,ray);\n\
				 \twhile(query.Proceed()){{}}\n\
				 \tif(query.CommittedStatus()!=COMMITTED_TRIANGLE_HIT){{{miss}}}\n\
				 \t{found}\n\
				 }}\n"
			);
		}
		if requirements.uses_trace_occlusion {
			// Any triangle answers the query, so traversal stops at the first one and reports nothing about it.
			let _ = write!(
				string,
				"bool _besl_trace_occlusion(RaytracingAccelerationStructure scene,uint mask,float3 origin,float3 direction,float t_min,float t_max){{\n\
				 \tRayQuery<RAY_FLAG_FORCE_OPAQUE|RAY_FLAG_ACCEPT_FIRST_HIT_AND_END_SEARCH> query;\n\
				 \tRayDesc ray;ray.Origin=origin;ray.Direction=direction;ray.TMin=t_min;ray.TMax=t_max;\n\
				 \tquery.TraceRayInline(scene,RAY_FLAG_NONE,mask,ray);\n\
				 \tquery.Proceed();\n\
				 \treturn query.CommittedStatus()==COMMITTED_TRIANGLE_HIT;\n\
				 }}\n"
			);
		}
	}

	/// Emits the call struct that carries `payload`, the type `invoke` passes or a closest hit returns, together with
	/// the hit it is shaded for. The payload crosses the call as [`CALL_PAYLOAD_STRUCT`], with conversions from and to
	/// `payload`. A ray generation shader also gets `_besl_invoke`, which runs the closest hit of a hit's record and
	/// returns the payload it leaves, or returns the payload of a miss unchanged. A closest hit gets its copy of the
	/// call, [`CALL_DATA`].
	///
	/// Call it right after the payload struct is emitted, so the struct is declared and every function that reads the
	/// call follows it.
	///
	/// Fails when a member of `payload` is a 16-bit array or a struct that holds 16-bit values, which the call can't
	/// carry widened.
	pub(crate) fn emit_call_data(&self, string: &mut String, payload: &str, fields: &[besl::NodeReference]) -> Result<(), ()> {
		let payload = Self::type_identifier(payload);
		let hit = besl::HIT_STRUCT;
		let break_str = ShaderFormatting::new(self.minified).break_str();

		// The widened struct, then the conversions, member by member.
		let mut members = String::new();
		let mut widen = String::new();
		let mut narrow = String::new();
		for field in fields {
			let field = field.borrow();
			let besl::Nodes::Member { name, r#type, count } = field.node() else {
				continue;
			};
			let type_name = r#type.borrow().get_name().ok_or(())?.to_owned();
			let member = Self::identifier(name).to_string();
			match widened_call_type(&type_name) {
				Some(_) if count.is_some() => return Err(()),
				Some(wide) => {
					let _ = write!(members, "{wide} {member};");
					let _ = write!(narrow, "value.{member}=({})wire.{member};", Self::translate_type(&type_name));
				}
				None if holds_16_bit_values(r#type) => return Err(()),
				None => {
					Self::emit_type_name(&mut members, &type_name);
					let _ = write!(members, " {member}");
					if let Some(count) = count {
						let _ = write!(members, "[{count}]");
					}
					members.push(';');
					let _ = write!(narrow, "value.{member}=wire.{member};");
				}
			}
			let _ = write!(widen, "wire.{member}=value.{member};");
		}
		let _ = write!(
			string,
			"struct {CALL_PAYLOAD_STRUCT}{{{members}}};{break_str}\
			 {CALL_PAYLOAD_STRUCT} {WIDEN_PAYLOAD}({payload} value){{{CALL_PAYLOAD_STRUCT} wire;{widen}return wire;}}{break_str}\
			 {payload} {NARROW_PAYLOAD}({CALL_PAYLOAD_STRUCT} wire){{{payload} value;{narrow}return value;}}{break_str}\
			 struct {CALL_STRUCT}{{{CALL_PAYLOAD_STRUCT} payload;{hit} hit;}};{break_str}"
		);
		match self.stage {
			Stages::RayGeneration => {
				let distance = Self::identifier("distance");
				let record = Self::identifier("record");
				// `>=` is false for a negative distance and for one that isn't a number, which are both misses. HLSL
				// reserves `payload`, so the parameter takes another name.
				let _ = write!(
					string,
					"{payload} _besl_invoke({hit} hit,{payload} value){{\n\
					 \tif(hit.{distance}>=0.0){{{CALL_STRUCT} data;data.payload={WIDEN_PAYLOAD}(value);data.hit=hit;CallShader(hit.{record},data);return {NARROW_PAYLOAD}(data.payload);}}\n\
					 \treturn value;\n\
					 }}\n"
				);
			}
			_ => {
				let _ = write!(string, "static {CALL_STRUCT} {CALL_DATA};{break_str}");
			}
		}
		Ok(())
	}

	/// Writes a ray generation shader's `main` as the DXR `raygeneration` entry point, which takes no parameters.
	pub(crate) fn emit_ray_generation_entry_point(&mut self, string: &mut String, statements: &[besl::NodeReference]) {
		string.push_str("[shader(\"raygeneration\")]");
		string.push_str(ShaderFormatting::new(self.minified).break_str());
		string.push_str("void ");
		Self::identifier("main").push_to(string);
		string.push('(');
		ShaderFormatting::new(self.minified).push_block_start(string);
		self.emit_function_statement_block(string, statements, 1);
		self.emit_block_end(string);
	}

	/// Writes a closest hit's `main` as a function that takes and returns the payload, then the `callable` entry point,
	/// which shades the payload the ray generation shader passed and leaves the result in the call it passed.
	///
	/// Fails when `main` doesn't take the payload as its only parameter.
	pub(crate) fn emit_closest_hit_entry_point(&mut self, string: &mut String, main: &besl::NodeReference) -> Result<(), ()> {
		// Entry normalization leaves the payload as the only parameter of a `main` that returns it. `main` stays a
		// function of its own, so its `return` statements keep returning the payload.
		{
			let node = main.borrow();
			let besl::Nodes::Function {
				params,
				return_type,
				statements,
				..
			} = node.node()
			else {
				return Err(());
			};
			if params.len() != 1 {
				return Err(());
			}
			Self::emit_type_name(string, return_type.borrow().get_name().ok_or(())?);
			let _ = write!(string, " {CLOSEST_HIT_MAIN}(");
			self.emit_function_parameters(string, params);
			ShaderFormatting::new(self.minified).push_block_start(string);
			self.emit_function_statement_block(string, statements, 1);
			self.emit_block_end(string);
		}
		let break_str = ShaderFormatting::new(self.minified).break_str();
		let _ = write!(string, "[shader(\"callable\")]{break_str}void ");
		Self::identifier("main").push_to(string);
		let _ = write!(
			string,
			"(inout {CALL_STRUCT} data){{{CALL_DATA}=data;{CALL_DATA}.payload={WIDEN_PAYLOAD}({CLOSEST_HIT_MAIN}({NARROW_PAYLOAD}({CALL_DATA}.payload)));data={CALL_DATA};}}{break_str}"
		);
		Ok(())
	}

	/// Writes the value of a closest hit's stage value `name`, such as `hit_distance`: a member of the hit it shades.
	pub(crate) fn emit_hit_value(string: &mut String, name: &str) {
		let _ = write!(string, "{CALL_DATA}.hit.{}", Self::identifier(hit_member(name)));
	}
}
