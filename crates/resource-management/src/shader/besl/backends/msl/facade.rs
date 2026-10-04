use std::collections::HashMap;

use super::{SUBGROUP_INTRINSICS, any_code_node, is_intrinsic_call};

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
pub struct Generator {
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
	/// The hidden kernel values each emitted function needs, analyzed once per shader before emission.
	pub(crate) hidden_contexts: HashMap<besl::NodeReference, HiddenContext>,
}

/// The `HiddenContext` struct records which hidden kernel values one function forwards, so that emitting its
/// declaration and every call to it doesn't walk its body and callees again.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct HiddenContext {
	/// The function reads a binding, push constant, workgroup value, or task payload, directly or through a callee.
	pub(crate) requires_resources: bool,
	/// The function reads the subgroup lane index, directly or through a callee.
	pub(crate) uses_simd_lane_id: bool,
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

/// The `MeshWrite` struct is one field write into a mesh vertex or primitive at `index`.
pub(crate) struct MeshWrite {
	/// Whether the write targets a vertex instead of a primitive.
	pub(crate) per_vertex: bool,
	pub(crate) field: String,
	pub(crate) index: besl::NodeReference,
	pub(crate) value: besl::NodeReference,
}

#[derive(Clone, Debug)]
pub(crate) struct MeshStageContext {
	pub(crate) has_resources: bool,
	pub(crate) has_push_constant: bool,
	pub(crate) has_task_payload: bool,
	pub(crate) uses_render_target_array_index: bool,
	/// Mesh output fields in declaration order, each with whether it belongs to `VertexOutput` instead of
	/// `PrimitiveOutput`.
	pub(crate) mesh_output_fields: Vec<(bool, String)>,
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
#[derive(Clone, Copy, Debug)]
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

#[derive(Default)]
pub(crate) struct ClassifiedNodes<'a> {
	pub(crate) bindings: Vec<&'a besl::NodeReference>,
	pub(crate) inputs: Vec<&'a besl::NodeReference>,
	pub(crate) outputs: Vec<&'a besl::NodeReference>,
	pub(crate) task_payloads: Vec<&'a besl::NodeReference>,
	pub(crate) workgroups: Vec<&'a besl::NodeReference>,
	pub(crate) declarations: Vec<&'a besl::NodeReference>,
	pub(crate) functions: Vec<&'a besl::NodeReference>,
	pub(crate) push_constant: Option<&'a besl::NodeReference>,
}

impl Generator {
	/// Creates an MSL transpiler with the default formatting mode.
	pub fn new() -> Self {
		Generator {
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
			hidden_contexts: HashMap::new(),
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

	/// Returns the hidden kernel values a function forwards, as analyzed for the shader being generated.
	pub(crate) fn hidden_context(&self, function: &besl::NodeReference) -> HiddenContext {
		self.hidden_contexts
			.get(function)
			.copied()
			.unwrap_or_else(|| analyze_hidden_context(function, &mut HashMap::new()))
	}
}

/// Analyzes every function in `order` once, so emitting each declaration and call site is a lookup.
pub(crate) fn analyze_hidden_contexts(order: &[besl::NodeReference]) -> HashMap<besl::NodeReference, HiddenContext> {
	let mut contexts = HashMap::with_capacity(order.len());
	for node in order {
		if matches!(node.borrow().node(), besl::Nodes::Function { .. }) {
			analyze_hidden_context(node, &mut contexts);
		}
	}
	contexts
}

/// Detects whether a function's reachable AST needs backend resource parameters or the subgroup lane index.
///
/// Callees are analyzed once and their results reused through `contexts`, so a call graph costs one walk per
/// function instead of one walk per call site.
fn analyze_hidden_context(
	function: &besl::NodeReference,
	contexts: &mut HashMap<besl::NodeReference, HiddenContext>,
) -> HiddenContext {
	if let Some(context) = contexts.get(function) {
		return *context;
	}
	// Shaders can't recurse, but a placeholder keeps a malformed call graph from looping.
	contexts.insert(function.clone(), HiddenContext::default());

	fn node_requires_resource_context(
		node: &besl::NodeReference,
		visited: &mut Vec<besl::NodeReference>,
		contexts: &mut HashMap<besl::NodeReference, HiddenContext>,
	) -> bool {
		if visited.iter().any(|visited_node| visited_node == node) {
			return false;
		}

		visited.push(node.clone());

		let mut visit = |child: &besl::NodeReference| node_requires_resource_context(child, visited, contexts);
		let result = match node.borrow().node() {
			besl::Nodes::Binding { .. }
			| besl::Nodes::TaskPayload { .. }
			| besl::Nodes::Workgroup { .. }
			| besl::Nodes::PushConstant { .. } => true,
			besl::Nodes::Scope { children, .. } | besl::Nodes::Struct { fields: children, .. } => {
				children.iter().any(&mut visit)
			}
			besl::Nodes::Function {
				params,
				return_type,
				statements,
				..
			} => params.iter().any(&mut visit) || visit(return_type) || statements.iter().any(&mut visit),
			branch @ (besl::Nodes::Conditional { .. } | besl::Nodes::Match { .. }) => branch.branch_children().any(&mut visit),
			besl::Nodes::ForLoop {
				initializer,
				condition,
				update,
				statements,
			} => visit(initializer) || visit(condition) || visit(update) || statements.iter().any(&mut visit),
			besl::Nodes::Raw { input, output, .. } => input.iter().chain(output).any(&mut visit),
			besl::Nodes::Parameter { r#type, .. }
			| besl::Nodes::Member { r#type, .. }
			| besl::Nodes::Specialization { r#type, .. }
			| besl::Nodes::Input { format: r#type, .. }
			| besl::Nodes::Output { format: r#type, .. } => visit(r#type),
			besl::Nodes::Expression(expression) => match expression {
				besl::Expressions::Operator { left, right, .. } | besl::Expressions::Accessor { left, right } => {
					visit(left) || visit(right)
				}
				// Calls use `contexts` directly, so this arm walks without the `visit` closure.
				besl::Expressions::FunctionCall {
					function, parameters, ..
				} => {
					let callee = function.get();
					let callee_requires = if matches!(callee.borrow().node(), besl::Nodes::Function { .. }) {
						analyze_hidden_context(&callee, contexts).requires_resources
					} else {
						node_requires_resource_context(&callee, visited, contexts)
					};
					callee_requires
						|| parameters
							.iter()
							.any(|parameter| node_requires_resource_context(parameter, visited, contexts))
				}
				besl::Expressions::IntrinsicCall { arguments, elements, .. } => {
					arguments.iter().chain(elements).any(&mut visit)
				}
				besl::Expressions::Expression { elements } => elements.iter().any(&mut visit),
				besl::Expressions::Macro { body, .. } => visit(body),
				besl::Expressions::Member { source, .. } => visit(source),
				besl::Expressions::VariableDeclaration { r#type, .. } => visit(r#type),
				besl::Expressions::Return { value } => value.as_ref().is_some_and(&mut visit),
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

	let requires_resources = node_requires_resource_context(function, &mut Vec::new(), contexts);
	// The lane index is a kernel builtin, so every caller on the path to its use must forward it.
	let uses_simd_lane_id = any_code_node(function, false, &mut |node| {
		if is_intrinsic_call(node, "subgroup_lane_index") {
			return true;
		}
		let callee = match node.borrow().node() {
			besl::Nodes::Expression(besl::Expressions::FunctionCall { function, .. }) => function.get(),
			_ => return false,
		};
		matches!(callee.borrow().node(), besl::Nodes::Function { .. })
			&& analyze_hidden_context(&callee, contexts).uses_simd_lane_id
	});
	let context = HiddenContext {
		requires_resources,
		uses_simd_lane_id,
	};
	contexts.insert(function.clone(), context);
	context
}
