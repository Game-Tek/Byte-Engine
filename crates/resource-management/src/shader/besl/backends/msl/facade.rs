use std::{
	alloc::{Allocator, Global},
	cell::RefCell,
	fmt::Write as _,
	vec::Vec,
};

pub use Generator as MSLTranspiler;

use super::*;
use crate::shader::generator::{
	NodeEmitter, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages, emit_comma_separated_nodes,
	emit_statement_block, ordered_shader_nodes_in,
};

/// The `Generator` struct exists to generate Metal Shading Language shaders from BESL ASTs.
///
/// Raster-stage IO uses conventional BESL names for Metal semantics. The implicit vertex values
/// `vertex_index` and `instance_index` are emitted as entry-point parameters with `[[vertex_id]]`
/// and `[[instance_id]]` instead of vertex-attribute struct fields. Fragment inputs named
/// `front_facing` are emitted as a `[[front_facing]]` entry-point parameter. Fragment outputs named
/// `depth`, `stencil`, and `sample_mask` are emitted with their matching Metal attributes; other
/// fragment outputs are emitted as color attachments by location. Fragment shaders may also return
/// an explicit output struct directly. Integer user varyings are emitted as `[[flat]]` user attributes.
///
/// # Parameters
///
/// - `minified`: Controls compact shader output. The default is `true` in release builds.
pub struct Generator<A: Allocator + Clone = Global> {
	pub(crate) allocator: A,
	pub(crate) minified: bool,
	pub(crate) compute_binding_mode: ComputeBindingMode,
	pub(crate) in_compute_body: bool,
	pub(crate) compute_stage_context: Option<ComputeStageContext>,
	pub(crate) raster_stage_context: Option<RasterStageContext>,
	pub(crate) task_stage_context: Option<TaskStageContext>,
	pub(crate) mesh_stage_context: Option<MeshStageContext>,
	pub(crate) in_buffer_binding_struct: bool,
	pub(crate) packed_mat4x3_members: Vec<besl::NodeReference>,
	pub(crate) match_break_depth: Option<usize>,
}

pub(crate) const PUSH_CONSTANT_BINDING_INDEX: u32 = 15;

/// Selects the Metal address space from the buffer's declared memory class and access mode.
pub(crate) fn buffer_address_space(memory_class: besl::BufferMemoryClass, write: bool) -> &'static str {
	match (memory_class, write) {
		(_, true) => "device",
		(besl::BufferMemoryClass::Constant, false) => "constant",
		(besl::BufferMemoryClass::Device, false) => "const device",
	}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ComputeBindingMode {
	ArgumentBuffers,
	BareResources,
}

#[derive(Clone, Debug)]
pub(crate) struct MeshStageContext {
	pub(crate) has_resources: bool,
	pub(crate) has_push_constant: bool,
	pub(crate) has_task_payload: bool,
	pub(crate) uses_render_target_array_index: bool,
	pub(crate) primitive_output_fields: Vec<String>,
	pub(crate) maximum_vertices: u32,
	pub(crate) maximum_primitives: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct TaskStageContext {
	pub(crate) has_resources: bool,
	pub(crate) has_push_constant: bool,
	pub(crate) has_task_payload: bool,
	pub(crate) workgroups: Vec<StageWorkgroup>,
}

#[derive(Clone, Debug)]
pub(crate) struct StageWorkgroup {
	pub(crate) name: String,
	pub(crate) msl_type: String,
	pub(crate) count: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct ComputeStageContext {
	pub(crate) has_resources: bool,
	pub(crate) has_push_constant: bool,
	pub(crate) workgroups: Vec<StageWorkgroup>,
}

/// The `RasterStageContext` struct carries the flat argument buffer into binding-dependent raster helpers.
#[derive(Clone, Debug)]
pub(crate) struct RasterStageContext {
	pub(crate) has_resources: bool,
	pub(crate) has_push_constant: bool,
	pub(crate) has_vertex_index: bool,
	pub(crate) has_instance_index: bool,
}

impl RasterStageContext {
	pub(crate) fn has_hidden_inputs(&self) -> bool {
		self.has_push_constant || self.has_vertex_index || self.has_instance_index
	}
}

/// The `IntrinsicRequirements` struct records the generated helpers and Metal builtins a shader needs.
#[derive(Default)]
pub(crate) struct IntrinsicRequirements {
	pub(crate) uses_atomic_compare_exchange: bool,
	pub(crate) uses_sincos: bool,
	pub(crate) uses_find_lsb: bool,
	pub(crate) uses_subgroup_intrinsics: bool,
	pub(crate) uses_simd_lane_id: bool,
	pub(crate) uses_downsample_min: bool,
	pub(crate) uses_downsample_max: bool,
	pub(crate) uses_render_target_array_index: bool,
}

pub(crate) struct ClassifiedNodes<'a, A: Allocator + Clone> {
	pub(crate) bindings: Vec<&'a besl::NodeReference, A>,
	pub(crate) inputs: Vec<&'a besl::NodeReference, A>,
	pub(crate) outputs: Vec<&'a besl::NodeReference, A>,
	pub(crate) task_payloads: Vec<&'a besl::NodeReference, A>,
	pub(crate) workgroups: Vec<&'a besl::NodeReference, A>,
	pub(crate) declarations: Vec<&'a besl::NodeReference, A>,
	pub(crate) functions: Vec<&'a besl::NodeReference, A>,
	pub(crate) push_constant: Option<&'a besl::NodeReference>,
}

impl<A: Allocator + Clone> ShaderGenerator for Generator<A> {}

impl Generator<Global> {
	/// Creates an MSL transpiler with the default formatting mode.
	pub fn new() -> Self {
		Self::new_in(Global)
	}
}

impl<A: Allocator + Clone> Generator<A> {
	/// Creates an MSL transpiler that uses `allocator` for temporary output buffers.
	pub fn new_in(allocator: A) -> Self {
		Generator {
			allocator,
			minified: !cfg!(debug_assertions), // Minify by default in release mode
			compute_binding_mode: ComputeBindingMode::ArgumentBuffers,
			in_compute_body: false,
			compute_stage_context: None,
			raster_stage_context: None,
			task_stage_context: None,
			mesh_stage_context: None,
			in_buffer_binding_struct: false,
			packed_mat4x3_members: Vec::new(),
			match_break_depth: None,
		}
	}

	pub fn minified(mut self, minified: bool) -> Self {
		self.minified = minified;
		self
	}

	pub fn compute_binding_mode(mut self, compute_binding_mode: ComputeBindingMode) -> Self {
		self.compute_binding_mode = compute_binding_mode;
		self
	}

	pub fn allocator(&self) -> &A {
		&self.allocator
	}

	/// Collects source requirements while walking emitted function bodies once instead of rescanning them for each helper.
	pub(crate) fn collect_intrinsic_requirements(order: &[besl::NodeReference]) -> IntrinsicRequirements {
		pub(crate) fn record(requirements: &mut IntrinsicRequirements, name: &str) {
			match name {
				"atomic_compare_exchange" => requirements.uses_atomic_compare_exchange = true,
				"sincos" => requirements.uses_sincos = true,
				"find_lsb" => requirements.uses_find_lsb = true,
				"subgroup_lane_index" => {
					requirements.uses_subgroup_intrinsics = true;
					requirements.uses_simd_lane_id = true;
				}
				name if SUBGROUP_INTRINSICS.contains(&name) => requirements.uses_subgroup_intrinsics = true,
				"downsample_min" => requirements.uses_downsample_min = true,
				"downsample_max" => requirements.uses_downsample_max = true,
				"set_mesh_primitive_render_target_array_index" => requirements.uses_render_target_array_index = true,
				_ => {}
			}
		}

		let mut requirements = IntrinsicRequirements::default();
		for node in order {
			any_code_node(node, false, &mut |node| {
				if let besl::Nodes::Expression(besl::Expressions::IntrinsicCall { intrinsic, .. }) = node.borrow().node()
					&& let Some(name) = intrinsic.borrow().get_name()
				{
					record(&mut requirements, name);
				}
				false
			});
		}
		requirements
	}

	/// Detects whether a function's reachable AST needs backend resource parameters.
	pub(crate) fn function_requires_resource_context(&self, function_node: &besl::NodeReference) -> bool {
		pub(crate) fn node_requires_resource_context<A: Allocator + Clone>(
			node: &besl::NodeReference,
			visited: &mut Vec<besl::NodeReference, A>,
		) -> bool {
			if visited.iter().any(|visited_node| visited_node == node) {
				return false;
			}

			visited.push(node.clone());

			let result = match node.borrow().node() {
				besl::Nodes::Binding { .. } => true,
				besl::Nodes::TaskPayload { .. } => true,
				besl::Nodes::Workgroup { .. } => true,
				besl::Nodes::PushConstant { .. } => true,
				besl::Nodes::Scope { children, .. } => {
					children.iter().any(|child| node_requires_resource_context(child, visited))
				}
				besl::Nodes::Function {
					params,
					return_type,
					statements,
					..
				} => {
					params.iter().any(|param| node_requires_resource_context(param, visited))
						|| node_requires_resource_context(return_type, visited)
						|| statements
							.iter()
							.any(|statement| node_requires_resource_context(statement, visited))
				}
				branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => branch
					.branch_children()
					.any(|child| node_requires_resource_context(child, visited)),
				besl::Nodes::ForLoop {
					initializer,
					condition,
					update,
					statements,
				} => {
					node_requires_resource_context(initializer, visited)
						|| node_requires_resource_context(condition, visited)
						|| node_requires_resource_context(update, visited)
						|| statements
							.iter()
							.any(|statement| node_requires_resource_context(statement, visited))
				}
				besl::Nodes::Struct { fields, .. } => fields.iter().any(|field| node_requires_resource_context(field, visited)),
				besl::Nodes::Raw { input, output, .. } => {
					input.iter().any(|input| node_requires_resource_context(input, visited))
						|| output.iter().any(|output| node_requires_resource_context(output, visited))
				}
				besl::Nodes::Parameter { r#type, .. }
				| besl::Nodes::Member { r#type, .. }
				| besl::Nodes::Specialization { r#type, .. }
				| besl::Nodes::Input { format: r#type, .. }
				| besl::Nodes::Output { format: r#type, .. } => node_requires_resource_context(r#type, visited),
				besl::Nodes::Expression(expression) => match expression {
					besl::Expressions::Operator { left, right, .. } => {
						node_requires_resource_context(left, visited) || node_requires_resource_context(right, visited)
					}
					besl::Expressions::FunctionCall {
						function, parameters, ..
					} => {
						node_requires_resource_context(&function.get(), visited)
							|| parameters
								.iter()
								.any(|parameter| node_requires_resource_context(parameter, visited))
					}
					besl::Expressions::IntrinsicCall { arguments, elements, .. } => {
						arguments
							.iter()
							.any(|argument| node_requires_resource_context(argument, visited))
							|| elements
								.iter()
								.any(|element| node_requires_resource_context(element, visited))
					}
					besl::Expressions::Expression { elements } => elements
						.iter()
						.any(|element| node_requires_resource_context(element, visited)),
					besl::Expressions::Macro { body, .. } => node_requires_resource_context(body, visited),
					besl::Expressions::Member { source, .. } => node_requires_resource_context(source, visited),
					besl::Expressions::VariableDeclaration { r#type, .. } => node_requires_resource_context(r#type, visited),
					besl::Expressions::Return { value } => value
						.as_ref()
						.is_some_and(|value| node_requires_resource_context(value, visited)),
					besl::Expressions::Accessor { left, right } => {
						node_requires_resource_context(left, visited) || node_requires_resource_context(right, visited)
					}
					besl::Expressions::Literal { .. }
					| besl::Expressions::Continue
					| besl::Expressions::Break
					| besl::Expressions::Discard => false,
				},
				_ => false,
			};

			visited.pop();
			result
		}

		node_requires_resource_context(function_node, &mut Vec::new_in(self.allocator.clone()))
	}
}
