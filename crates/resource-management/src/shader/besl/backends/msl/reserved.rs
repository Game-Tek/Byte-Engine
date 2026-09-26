//! MSL names that a BESL identifier must not take after lowering.
//!
//! The MSL [`Generator`](super::Generator) consults [`is_reserved`] through
//! [`NodeEmitter::is_reserved_identifier`](crate::shader::generator::NodeEmitter::is_reserved_identifier) and prefixes
//! matching names with [`RESERVED_IDENTIFIER_PREFIX`](crate::shader::generator::RESERVED_IDENTIFIER_PREFIX).
//! Metal Shading Language is based on C++14 and imports `metal_stdlib` with `using namespace metal`, so C++ keywords,
//! Metal keywords, Metal types, and the standard-library functions the backend calls are all reserved.

/// Reports whether `name` collides with a C++14 or MSL keyword, a Metal built-in type or function, or a name the
/// MSL backend declares itself.
pub(super) fn is_reserved(name: &str) -> bool {
	is_keyword(name) || is_numeric_type(name) || is_builtin_type(name) || is_builtin_function(name) || is_backend_name(name)
}

/// Keywords and alternative tokens from C++14, plus the MSL address spaces, function qualifiers, and namespaces.
fn is_keyword(name: &str) -> bool {
	matches!(
		name,
		// C++14 keywords.
		"alignas" | "alignof" | "asm" | "auto" | "bool" | "break" | "case" | "catch" | "char" | "char16_t" | "char32_t"
			| "class" | "const" | "constexpr" | "const_cast" | "continue" | "decltype" | "default" | "delete" | "do"
			| "double" | "dynamic_cast" | "else" | "enum" | "explicit" | "export" | "extern" | "false" | "float" | "for"
			| "friend" | "goto" | "if" | "inline" | "int" | "long" | "mutable" | "namespace" | "new" | "noexcept"
			| "nullptr" | "operator" | "private" | "protected" | "public" | "register" | "reinterpret_cast" | "return"
			| "short" | "signed" | "sizeof" | "static" | "static_assert" | "static_cast" | "struct" | "switch"
			| "template" | "this" | "thread_local" | "throw" | "true" | "try" | "typedef" | "typeid" | "typename"
			| "union" | "unsigned" | "using" | "virtual" | "void" | "volatile" | "wchar_t" | "while" | "override"
			| "final"
			// C++ alternative operator tokens.
			| "and" | "and_eq" | "bitand" | "bitor" | "compl" | "not" | "not_eq" | "or" | "or_eq" | "xor" | "xor_eq"
			// MSL address spaces and qualifiers.
			| "device" | "constant" | "thread" | "threadgroup" | "threadgroup_imageblock" | "ray_data" | "object_data"
			| "kernel" | "vertex" | "fragment" | "mesh" | "object" | "visible" | "intersection" | "stitchable"
			| "stage_in" | "patch"
			// Namespaces and enumerations the generated source names without a `metal::` qualifier.
			| "metal" | "std" | "access" | "component" | "topology" | "mem_flags" | "memory_order" | "memory_order_relaxed"
	)
}

/// Scalar, vector, and matrix type names such as `half`, `float3`, `packed_ushort4`, or `float4x3`.
///
/// Metal spells every vector and matrix as a scalar name followed by a size, so this check recognizes the pattern
/// instead of listing each spelling.
fn is_numeric_type(name: &str) -> bool {
	let unpacked = name.strip_prefix("packed_").unwrap_or(name);
	// Accept the scalar itself, a vector size, or a matrix `CxR` size.
	let is_dimension = |value: &str| matches!(value, "2" | "3" | "4");
	let is_size = |size: &str| match size.split_once('x') {
		Some((columns, rows)) => is_dimension(columns) && is_dimension(rows),
		None => size.is_empty() || is_dimension(size),
	};
	[
		"bool",
		"char",
		"uchar",
		"short",
		"ushort",
		"int",
		"uint",
		"long",
		"ulong",
		"half",
		"float",
		"bfloat",
		"double",
		"size_t",
		"ptrdiff_t",
		"int8_t",
		"uint8_t",
		"int16_t",
		"uint16_t",
		"int32_t",
		"uint32_t",
		"int64_t",
		"uint64_t",
		"intptr_t",
		"uintptr_t",
	]
	.iter()
	.any(|scalar| unpacked.strip_prefix(scalar).is_some_and(is_size))
}

/// Opaque, atomic, and SIMD-group type names from `metal_stdlib`.
fn is_builtin_type(name: &str) -> bool {
	matches!(
		name,
		"texture1d"
			| "texture1d_array"
			| "texture2d"
			| "texture2d_array"
			| "texture2d_ms"
			| "texture2d_ms_array"
			| "texture3d"
			| "texturecube"
			| "texturecube_array"
			| "texture_buffer"
			| "depth2d"
			| "depth2d_array"
			| "depth2d_ms"
			| "depth2d_ms_array"
			| "depthcube"
			| "depthcube_array"
			| "sampler"
			| "array" | "array_ref"
			| "vec" | "matrix"
			| "packed_vec"
			| "atomic"
			| "atomic_int"
			| "atomic_uint"
			| "atomic_bool"
			| "atomic_float"
			| "atomic_ulong"
			| "imageblock"
			| "visible_function_table"
			| "intersection_function_table"
			| "acceleration_structure"
			| "primitive_acceleration_structure"
			| "instance_acceleration_structure"
			| "intersector"
			| "ray" | "simd_vote"
			| "simdgroup_matrix"
			| "simdgroup_float8x8"
			| "simdgroup_half8x8"
			| "mesh_grid_properties"
			| "render_grid_properties"
	)
}

/// `metal_stdlib` functions and member functions the MSL backend calls when it lowers BESL intrinsics.
///
/// A BESL local with one of these names would shadow the function at the call site.
fn is_builtin_function(name: &str) -> bool {
	matches!(
		name,
		"abs" | "acos" | "acosh" | "all" | "any" | "asin" | "asinh" | "atan" | "atan2" | "atanh" | "ceil" | "clamp"
			| "copysign" | "cos" | "cosh" | "cross" | "ctz" | "clz" | "degrees" | "determinant" | "distance" | "dot"
			| "exp" | "exp2" | "exp10" | "fabs" | "floor" | "fma" | "fmax" | "fmin" | "fmod" | "fract" | "fwidth"
			| "dfdx" | "dfdy" | "isfinite" | "isinf" | "isnan" | "isnormal" | "length" | "log" | "log2" | "log10"
			| "max" | "min" | "mix" | "normalize" | "popcount" | "pow" | "powr" | "radians" | "reflect" | "refract"
			| "rint" | "round" | "rsqrt" | "saturate" | "select" | "sign" | "sin" | "sincos" | "sinh" | "smoothstep"
			| "sqrt" | "step" | "tan" | "tanh" | "transpose" | "trunc" | "as_type" | "discard_fragment"
			| "threadgroup_barrier" | "simdgroup_barrier" | "simd_ballot" | "simd_broadcast" | "simd_shuffle"
			| "atomic_load_explicit" | "atomic_store_explicit" | "atomic_exchange_explicit"
			| "atomic_compare_exchange_weak_explicit" | "atomic_fetch_add_explicit" | "atomic_fetch_sub_explicit"
			| "atomic_fetch_min_explicit" | "atomic_fetch_max_explicit" | "atomic_fetch_and_explicit"
			| "atomic_fetch_or_explicit" | "atomic_fetch_xor_explicit"
			// Texture and mesh member functions.
			| "sample" | "read" | "write" | "gather" | "get_width" | "get_height" | "get_depth" | "get_num_mip_levels"
			| "set_vertex" | "set_primitive" | "set_index" | "set_primitive_count" | "set_threadgroups_per_grid"
	)
}

/// Names the MSL backend declares next to user code: helpers, interface structs, and entry-point parameters.
///
/// Some backend names are absent on purpose because BESL code must reach them by the same name:
/// - `main`: the backend renames the BESL entry point to [`MSL_ENTRY_POINT`](super::MSL_ENTRY_POINT) itself.
/// - `push_constant`: the BESL push-constant block lowers to the backend parameter of that name.
/// - `FragmentOutput`: a fragment `main` may return a BESL struct of that name as its explicit output.
/// - `vertex_index`, `instance_index`, and `front_facing`: the backend binds these raster builtins by their BESL names.
///
/// `level` and `gradient2d` are absent because the backend always writes them as `metal::level` and
/// `metal::gradient2d`.
fn is_backend_name(name: &str) -> bool {
	matches!(
		name,
		"PI" | "PushConstant"
			| "ObjectPayload"
			| "payload"
			| "VertexInput"
			| "VertexOutput"
			| "FragmentInput"
			| "PrimitiveOutput"
			| "_resources"
			| "resources"
			| "in" | "out"
			| "gid" | "thread_index"
			| "thread_position"
			| "threadgroup_position"
			| "simd_lane_id"
			| "mesh_grid"
			| "out_mesh"
			| "_besl_downsample_min"
			| "_besl_downsample_max"
			| "_besl_packed_float4x3"
			| "_besl_load_mat4x3"
			| "_besl_pack_mat4x3"
			| "_besl_store_mat4x3"
			| "_besl_atomic_compare_exchange"
			| "_besl_sincos"
			| "_besl_find_lsb"
			| "_besl_subgroup_ballot"
			| "_besl_subgroup_ballot_any"
			| "_besl_subgroup_ballot_find_lsb"
			| "_besl_subgroup_ballot_count"
			| "_besl_subgroup_ballot_and_not"
			| "_besl_subgroup_broadcast_u32"
			| "_besl_subgroup_broadcast_f32"
			| "_besl_triangle_index"
			| "_besl_triangle"
	)
}
