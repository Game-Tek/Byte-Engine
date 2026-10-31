//! Lowers ray generation and closest-hit shaders, and the ray queries compute and ray generation shaders run, to GLSL
//! for Vulkan.
//!
//! The lowering follows the contract between the Vulkan GHI backend and the shaders it links, which the ray tracing
//! design note (`docs/develop/rendering/ray-tracing.mdx`) describes. A ray generation shader is a `raygen` shader
//! that finds hits with ray queries and runs closest hits with `executeCallableEXT`, and a closest hit is a `callable`
//! shader. The two exchange [`CALL_DATA`]: the payload and the hit it is shaded for. Resources need no plumbing,
//! because every stage of a pipeline reaches the same descriptor bindings.

use std::fmt::Write as _;

use super::super::super::IntrinsicRequirements;
use super::Generator;
use crate::shader::generator::{NodeEmitter, ShaderFormatting, Stages};

/// Names the struct a ray generation shader and the closest hits it runs exchange: the payload, then the hit.
pub(crate) const CALL_STRUCT: &str = "_besl_call";

/// Names the callable data of a ray tracing stage, which holds one [`CALL_STRUCT`].
pub(crate) const CALL_DATA: &str = "_besl_call_data";

/// Names the function that runs a closest hit's `main`, which the callable entry point calls.
pub(crate) const CLOSEST_HIT_MAIN: &str = "_besl_closest_hit_main";

impl Generator {
	/// Reports whether the shader needs GLSL's ray tracing support: it is a ray tracing stage, runs ray queries, or
	/// declares a `Hit` or an acceleration structure.
	pub(super) fn uses_ray_tracing(stage: Stages, order: &[besl::NodeReference], uses_ray_queries: bool) -> bool {
		stage.is_ray_tracing() || uses_ray_queries || order.iter().any(super::super::super::declares_ray_tracing)
	}

	/// Reports whether the shader needs ray queries: it declares an acceleration structure, or tests rays against one.
	pub(super) fn uses_ray_queries(order: &[besl::NodeReference], requirements: &IntrinsicRequirements) -> bool {
		requirements.uses_intersect
			|| requirements.uses_trace_occlusion
			|| order.iter().any(|node| {
				matches!(
					node.borrow().node(),
					besl::Nodes::Binding {
						r#type: besl::BindingTypes::AccelerationStructure,
						..
					}
				)
			})
	}

	/// Emits what a shader that uses ray tracing declares before its own code: the `Hit` struct and the helpers of the
	/// ray queries the shader runs.
	///
	/// The shader's own emission skips the `Hit` node, so the struct is declared once, before the helpers return it.
	pub(super) fn emit_ray_tracing_prelude(&self, string: &mut String, requirements: &IntrinsicRequirements) {
		self.emit_named_struct_start(string, besl::HIT_STRUCT);
		for (name, r#type) in besl::HIT_MEMBERS {
			self.emit_indentation(string, 1);
			Self::emit_type_name(string, r#type);
			string.push(' ');
			Self::identifier(name).push_to(string);
			self.emit_statement_end(string);
		}
		self.emit_struct_declaration_end(string);

		let hit = besl::HIT_STRUCT;
		if requirements.uses_intersect {
			// Every triangle is opaque, so the query commits the nearest triangle without asking the shader, and the loop
			// that would confirm candidates runs no iteration. A hit's record is its instance's shader binding table offset
			// plus its geometry's index, which is what every backend computes.
			let _ = write!(
				string,
				"{hit} _besl_intersect(accelerationStructureEXT scene,uint mask,vec3 origin,vec3 direction,float t_min,float t_max){{\n\
				 \trayQueryEXT query;\n\
				 \trayQueryInitializeEXT(query,scene,gl_RayFlagsOpaqueEXT,mask,origin,t_min,direction,t_max);\n\
				 \twhile(rayQueryProceedEXT(query)){{}}\n\
				 \tif(rayQueryGetIntersectionTypeEXT(query,true)==gl_RayQueryCommittedIntersectionNoneEXT){{return {hit}(0u,-1.0,0u,0u,0u,vec2(0.0),origin,direction);}}\n\
				 \tuint geometry=uint(rayQueryGetIntersectionGeometryIndexEXT(query,true));\n\
				 \treturn {hit}(rayQueryGetIntersectionInstanceShaderBindingTableRecordOffsetEXT(query,true)+geometry,rayQueryGetIntersectionTEXT(query,true),uint(rayQueryGetIntersectionInstanceCustomIndexEXT(query,true)),geometry,uint(rayQueryGetIntersectionPrimitiveIndexEXT(query,true)),rayQueryGetIntersectionBarycentricsEXT(query,true),origin,direction);\n\
				 }}\n"
			);
		}
		if requirements.uses_trace_occlusion {
			// Any triangle answers the query, so traversal stops at the first one and reports nothing about it.
			string.push_str(
				"bool _besl_trace_occlusion(accelerationStructureEXT scene,uint mask,vec3 origin,vec3 direction,float t_min,float t_max){\n\
				 \trayQueryEXT query;\n\
				 \trayQueryInitializeEXT(query,scene,gl_RayFlagsOpaqueEXT|gl_RayFlagsTerminateOnFirstHitEXT,mask,origin,t_min,direction,t_max);\n\
				 \trayQueryProceedEXT(query);\n\
				 \treturn rayQueryGetIntersectionTypeEXT(query,true)!=gl_RayQueryCommittedIntersectionNoneEXT;\n\
				 }\n",
			);
		}
	}

	/// Emits the callable data that carries `payload`, the type `invoke` passes or a closest hit returns, together with
	/// the hit it is shaded for. A ray generation shader also gets `_besl_invoke`, which runs the closest hit of a hit's
	/// record and returns the payload it leaves, or returns the payload of a miss unchanged.
	///
	/// Call it right after the payload struct is emitted, so the struct is declared and every function that reads the
	/// callable data follows it.
	pub(super) fn emit_call_data(&self, string: &mut String, payload: &str) {
		let payload = Self::type_identifier(payload);
		let hit = besl::HIT_STRUCT;
		let break_str = ShaderFormatting::new(self.minified).break_str();
		let _ = write!(string, "struct {CALL_STRUCT}{{{payload} payload;{hit} hit;}};{break_str}");
		match self.stage {
			Stages::RayGeneration => {
				let distance = Self::identifier("distance");
				// `>=` is false for a negative distance and for one that isn't a number, which are both misses.
				let _ = write!(
					string,
					"layout(location=0) callableDataEXT {CALL_STRUCT} {CALL_DATA};{break_str}\
					 {payload} _besl_invoke({hit} hit,{payload} payload){{\n\
					 \tif(hit.{distance}>=0.0){{{CALL_DATA}.payload=payload;{CALL_DATA}.hit=hit;executeCallableEXT(hit.record,0);return {CALL_DATA}.payload;}}\n\
					 \treturn payload;\n\
					 }}\n"
				);
			}
			_ => {
				let _ = write!(
					string,
					"layout(location=0) callableDataInEXT {CALL_STRUCT} {CALL_DATA};{break_str}"
				);
			}
		}
	}

	/// Writes a closest hit's `main` as a function that takes and returns the payload, then the callable entry point,
	/// which shades the payload the ray generation shader passed and leaves the result for it.
	///
	/// Fails when `main` doesn't take the payload as its only parameter.
	pub(super) fn emit_closest_hit_entry_point(&mut self, string: &mut String, main: &besl::NodeReference) -> Result<(), ()> {
		// Entry normalization leaves the payload as the only parameter of a `main` that returns it. `main` stays a
		// function of its own, so its `return` statements keep returning the payload.
		self.emit_main_as(string, main, CLOSEST_HIT_MAIN, 1)?;
		let _ = write!(
			string,
			"void main(){{{CALL_DATA}.payload={CLOSEST_HIT_MAIN}({CALL_DATA}.payload);}}{}",
			ShaderFormatting::new(self.minified).break_str()
		);
		Ok(())
	}

	/// Writes the value of a closest hit's stage value `name`, such as `hit_distance`: a member of the hit it shades.
	pub(super) fn emit_hit_value(string: &mut String, name: &str) {
		let member = super::super::super::hit_member(name);
		let _ = write!(string, "{CALL_DATA}.hit.{}", Self::identifier(member));
	}
}
