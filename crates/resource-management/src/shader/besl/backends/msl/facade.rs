use std::{
	alloc::{Allocator, Global},
	cell::RefCell,
	fmt::Write as _,
	vec::Vec,
};

pub use Generator as MSLShaderGenerator;

use super::*;
use crate::shader::generator::{
	MatrixLayouts, NodeEmitter, ShaderFormatting, ShaderGenerationSettings, ShaderGenerator, Stages,
	emit_comma_separated_nodes, emit_statement_block, ordered_shader_nodes_in,
};

/// The `Generator` struct exists to generate Metal Shading Language shaders from BESL ASTs.
///
/// Raster-stage IO uses conventional BESL names for Metal semantics. Vertex inputs named
/// `vertex_id` and `instance_id` are emitted as entry-point parameters with `[[vertex_id]]` and
/// `[[instance_id]]` instead of vertex-attribute struct fields. Fragment inputs named
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
	pub(crate) downsample_strategy: DownsampleStrategy,
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

/// Selects how BESL conservative 2x2 downsampling is implemented in MSL.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DownsampleStrategy {
	/// Gather four texels and reduce them in shader code for Metal targets without sampler reduction.
	ShaderGather,
	/// Use the texture's min/max reduction sampler. This is the default because engine depth-pyramid samplers require reduction support.
	NativeSamplerReduction,
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
}

/// The `IntrinsicRequirements` struct records the generated helpers and Metal builtins a shader needs.
#[derive(Default)]
pub(crate) struct IntrinsicRequirements {
	pub(crate) uses_atomic_compare_exchange: bool,
	pub(crate) uses_sincos: bool,
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
	/// Creates an MSL generator with the default formatting mode.
	pub fn new() -> Self {
		Self::new_in(Global)
	}
}

impl<A: Allocator + Clone> Generator<A> {
	/// Creates an MSL generator that uses `allocator` for temporary output buffers.
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
			downsample_strategy: DownsampleStrategy::NativeSamplerReduction,
		}
	}

	/// Selects the MSL implementation for `downsample_min` and `downsample_max`.
	///
	/// Select [`DownsampleStrategy::ShaderGather`] only for targets without hardware min/max sampler reduction.
	/// The bound sampler must use the matching reduction mode.
	pub fn downsample_strategy(mut self, strategy: DownsampleStrategy) -> Self {
		self.downsample_strategy = strategy;
		self
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

	/// Reports whether one reachable AST branch uses the requested intrinsic.
	pub(crate) fn uses_intrinsic(node: &besl::NodeReference, intrinsic_name: &str) -> bool {
		match node.borrow().node() {
			besl::Nodes::Function { statements, .. } => statements
				.iter()
				.any(|statement| Self::uses_intrinsic(statement, intrinsic_name)),
			besl::Nodes::Conditional { condition, statements } => {
				Self::uses_intrinsic(condition, intrinsic_name)
					|| statements
						.iter()
						.any(|statement| Self::uses_intrinsic(statement, intrinsic_name))
			}
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => {
				Self::uses_intrinsic(initializer, intrinsic_name)
					|| Self::uses_intrinsic(condition, intrinsic_name)
					|| Self::uses_intrinsic(update, intrinsic_name)
					|| statements
						.iter()
						.any(|statement| Self::uses_intrinsic(statement, intrinsic_name))
			}
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::IntrinsicCall {
					intrinsic, arguments, ..
				} => {
					intrinsic.borrow().get_name() == Some(intrinsic_name)
						|| arguments
							.iter()
							.any(|argument| Self::uses_intrinsic(argument, intrinsic_name))
				}
				besl::Expressions::Operator { left, right, .. } => {
					Self::uses_intrinsic(left, intrinsic_name) || Self::uses_intrinsic(right, intrinsic_name)
				}
				besl::Expressions::FunctionCall {
					function, parameters, ..
				} => {
					Self::uses_intrinsic(function, intrinsic_name)
						|| parameters
							.iter()
							.any(|parameter| Self::uses_intrinsic(parameter, intrinsic_name))
				}
				besl::Expressions::Expression { elements } => {
					elements.iter().any(|element| Self::uses_intrinsic(element, intrinsic_name))
				}
				besl::Expressions::Macro { body, .. } => Self::uses_intrinsic(body, intrinsic_name),
				besl::Expressions::Member { source, .. } => Self::uses_intrinsic(source, intrinsic_name),
				besl::Expressions::Return { value } => value
					.as_ref()
					.is_some_and(|value| Self::uses_intrinsic(value, intrinsic_name)),
				besl::Expressions::Accessor { left, right } => {
					Self::uses_intrinsic(left, intrinsic_name) || Self::uses_intrinsic(right, intrinsic_name)
				}
				besl::Expressions::VariableDeclaration { .. }
				| besl::Expressions::Literal { .. }
				| besl::Expressions::Continue
				| besl::Expressions::Discard => false,
			},
			_ => false,
		}
	}

	/// Collects source requirements while walking emitted function bodies once instead of rescanning them for each helper.
	pub(crate) fn collect_intrinsic_requirements(order: &[besl::NodeReference]) -> IntrinsicRequirements {
		pub(crate) fn record(requirements: &mut IntrinsicRequirements, name: &str) {
			match name {
				"atomic_compare_exchange" => requirements.uses_atomic_compare_exchange = true,
				"sincos" => requirements.uses_sincos = true,
				"subgroup_lane_index" => {
					requirements.uses_subgroup_intrinsics = true;
					requirements.uses_simd_lane_id = true;
				}
				"subgroup_ballot"
				| "subgroup_ballot_any"
				| "subgroup_ballot_find_lsb"
				| "subgroup_ballot_count"
				| "subgroup_ballot_and_not"
				| "subgroup_broadcast_u32"
				| "subgroup_broadcast_f32" => requirements.uses_subgroup_intrinsics = true,
				"downsample_min" => requirements.uses_downsample_min = true,
				"downsample_max" => requirements.uses_downsample_max = true,
				"set_mesh_primitive_render_target_array_index" => requirements.uses_render_target_array_index = true,
				_ => {}
			}
		}

		pub(crate) fn visit(node: &besl::NodeReference, requirements: &mut IntrinsicRequirements) {
			match node.borrow().node() {
				besl::Nodes::Function { statements, .. } => {
					for statement in statements {
						visit(statement, requirements);
					}
				}
				besl::Nodes::Conditional { condition, statements } => {
					visit(condition, requirements);
					for statement in statements {
						visit(statement, requirements);
					}
				}
				besl::Nodes::ForLoop {
					initializer,
					condition,
					update,
					statements,
				} => {
					visit(initializer, requirements);
					visit(condition, requirements);
					visit(update, requirements);
					for statement in statements {
						visit(statement, requirements);
					}
				}
				besl::Nodes::Expression(expression) => match expression {
					besl::Expressions::IntrinsicCall {
						intrinsic, arguments, ..
					} => {
						if let Some(name) = intrinsic.borrow().get_name() {
							record(requirements, name);
						}
						for argument in arguments {
							visit(argument, requirements);
						}
					}
					besl::Expressions::Operator { left, right, .. } | besl::Expressions::Accessor { left, right } => {
						visit(left, requirements);
						visit(right, requirements);
					}
					besl::Expressions::FunctionCall { parameters, .. } => {
						for parameter in parameters {
							visit(parameter, requirements);
						}
					}
					besl::Expressions::Expression { elements } => {
						for element in elements {
							visit(element, requirements);
						}
					}
					besl::Expressions::Macro { body, .. } => visit(body, requirements),
					besl::Expressions::Member { source, .. } => visit(source, requirements),
					besl::Expressions::Return { value } => {
						if let Some(value) = value {
							visit(value, requirements);
						}
					}
					besl::Expressions::VariableDeclaration { .. }
					| besl::Expressions::Literal { .. }
					| besl::Expressions::Continue
					| besl::Expressions::Discard => {}
				},
				_ => {}
			}
		}

		let mut requirements = IntrinsicRequirements::default();
		for node in order {
			visit(node, &mut requirements);
		}
		requirements
	}

	/// Detects whether a function's reachable AST needs backend resource parameters.
	pub(crate) fn function_requires_resource_context(
		&self,
		function_node: &besl::NodeReference,
		include_push_constant: bool,
	) -> bool {
		pub(crate) fn node_requires_resource_context<A: Allocator + Clone>(
			node: &besl::NodeReference,
			visited: &mut Vec<besl::NodeReference, A>,
			include_push_constant: bool,
		) -> bool {
			if visited.iter().any(|visited_node| visited_node == node) {
				return false;
			}

			visited.push(node.clone());

			let result = match node.borrow().node() {
				besl::Nodes::Binding { .. } => true,
				besl::Nodes::TaskPayload { .. } => true,
				besl::Nodes::Workgroup { .. } => true,
				besl::Nodes::PushConstant { .. } => include_push_constant,
				besl::Nodes::Scope { children, .. } => children
					.iter()
					.any(|child| node_requires_resource_context(child, visited, include_push_constant)),
				besl::Nodes::Function {
					params,
					return_type,
					statements,
					..
				} => {
					params
						.iter()
						.any(|param| node_requires_resource_context(param, visited, include_push_constant))
						|| node_requires_resource_context(return_type, visited, include_push_constant)
						|| statements
							.iter()
							.any(|statement| node_requires_resource_context(statement, visited, include_push_constant))
				}
				besl::Nodes::Conditional { condition, statements } => {
					node_requires_resource_context(condition, visited, include_push_constant)
						|| statements
							.iter()
							.any(|statement| node_requires_resource_context(statement, visited, include_push_constant))
				}
				besl::Nodes::ForLoop {
					initializer,
					condition,
					update,
					statements,
				} => {
					node_requires_resource_context(initializer, visited, include_push_constant)
						|| node_requires_resource_context(condition, visited, include_push_constant)
						|| node_requires_resource_context(update, visited, include_push_constant)
						|| statements
							.iter()
							.any(|statement| node_requires_resource_context(statement, visited, include_push_constant))
				}
				besl::Nodes::Struct { fields, .. } => fields
					.iter()
					.any(|field| node_requires_resource_context(field, visited, include_push_constant)),
				besl::Nodes::Raw { input, output, .. } => {
					input
						.iter()
						.any(|input| node_requires_resource_context(input, visited, include_push_constant))
						|| output
							.iter()
							.any(|output| node_requires_resource_context(output, visited, include_push_constant))
				}
				besl::Nodes::Parameter { r#type, .. }
				| besl::Nodes::Member { r#type, .. }
				| besl::Nodes::Specialization { r#type, .. }
				| besl::Nodes::Input { format: r#type, .. }
				| besl::Nodes::Output { format: r#type, .. } => node_requires_resource_context(r#type, visited, include_push_constant),
				besl::Nodes::Expression(expression) => match expression {
					besl::Expressions::Operator { left, right, .. } => {
						node_requires_resource_context(left, visited, include_push_constant)
							|| node_requires_resource_context(right, visited, include_push_constant)
					}
					besl::Expressions::FunctionCall {
						function, parameters, ..
					} => {
						node_requires_resource_context(function, visited, include_push_constant)
							|| parameters
								.iter()
								.any(|parameter| node_requires_resource_context(parameter, visited, include_push_constant))
					}
					besl::Expressions::IntrinsicCall { arguments, elements, .. } => {
						arguments
							.iter()
							.any(|argument| node_requires_resource_context(argument, visited, include_push_constant))
							|| elements
								.iter()
								.any(|element| node_requires_resource_context(element, visited, include_push_constant))
					}
					besl::Expressions::Expression { elements } => elements
						.iter()
						.any(|element| node_requires_resource_context(element, visited, include_push_constant)),
					besl::Expressions::Macro { body, .. } => {
						node_requires_resource_context(body, visited, include_push_constant)
					}
					besl::Expressions::Member { source, .. } => {
						node_requires_resource_context(source, visited, include_push_constant)
					}
					besl::Expressions::VariableDeclaration { r#type, .. } => {
						node_requires_resource_context(r#type, visited, include_push_constant)
					}
					besl::Expressions::Return { value } => value
						.as_ref()
						.is_some_and(|value| node_requires_resource_context(value, visited, include_push_constant)),
					besl::Expressions::Accessor { left, right } => {
						node_requires_resource_context(left, visited, include_push_constant)
							|| node_requires_resource_context(right, visited, include_push_constant)
					}
					besl::Expressions::Literal { .. } | besl::Expressions::Continue | besl::Expressions::Discard => false,
				},
				_ => false,
			};

			visited.pop();
			result
		}

		node_requires_resource_context(function_node, &mut Vec::new_in(self.allocator.clone()), include_push_constant)
	}
}
