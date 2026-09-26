//! HLSL names that a BESL identifier must not take after lowering.
//!
//! The HLSL [`Generator`](super::Generator) consults [`is_reserved`] through
//! [`NodeEmitter::is_reserved_identifier`](crate::shader::generator::NodeEmitter::is_reserved_identifier) and prefixes
//! matching names with [`RESERVED_IDENTIFIER_PREFIX`](crate::shader::generator::RESERVED_IDENTIFIER_PREFIX).

/// Reports whether `name` collides with an HLSL keyword, reserved word, built-in type or function, or a name the
/// HLSL backend declares itself.
pub(super) fn is_reserved(name: &str) -> bool {
	is_keyword(name) || is_numeric_type(name) || is_object_type(name) || is_builtin_function(name) || is_backend_name(name)
}

/// Keywords and reserved words that DXC rejects as identifiers, including the C++ words HLSL 2021 reserves and
/// the legacy effect-framework words.
fn is_keyword(name: &str) -> bool {
	matches!(
		name,
		// Language keywords and storage, interpolation, and matrix-packing modifiers.
		"break" | "case" | "cbuffer" | "centroid" | "class" | "column_major" | "const" | "continue" | "default"
			| "discard" | "do" | "else" | "export" | "extern" | "false" | "for" | "globallycoherent" | "groupshared"
			| "if" | "in" | "inline" | "inout" | "interface" | "linear" | "namespace" | "nointerpolation"
			| "noperspective" | "out" | "packoffset" | "precise" | "register" | "return" | "row_major" | "sample"
			| "shared" | "snorm" | "static" | "struct" | "switch" | "tbuffer" | "true" | "typedef" | "uniform"
			| "unorm" | "unsigned" | "volatile" | "while" | "NULL"
			// Geometry shader primitive types. The mesh modifiers `vertices`, `indices`, `primitives`, and `payload`
			// are contextual and stay valid identifiers.
			| "point" | "line" | "lineadj" | "triangle" | "triangleadj"
			// Scalar and generic type keywords whose sized forms `is_numeric_type` covers.
			| "void" | "vector" | "matrix" | "string"
			// C++ words that DXC reserves.
			| "auto" | "catch" | "char" | "const_cast" | "delete" | "dynamic_cast" | "enum" | "explicit" | "friend"
			| "goto" | "long" | "mutable" | "new" | "operator" | "private" | "protected" | "public"
			| "reinterpret_cast" | "short" | "signed" | "sizeof" | "static_cast" | "template" | "this" | "throw"
			| "try" | "typename" | "union" | "using" | "virtual" | "nullptr"
			// Legacy effect-framework words.
			| "asm" | "asm_fragment" | "compile" | "compile_fragment" | "CompileShader" | "fxgroup" | "pass"
			| "pixelfragment" | "vertexfragment" | "stateblock" | "stateblock_state" | "technique" | "technique10"
			| "technique11" | "BlendState" | "DepthStencilState" | "RasterizerState" | "DepthStencilView"
			| "RenderTargetView" | "ComputeShader" | "DomainShader" | "GeometryShader" | "HullShader" | "PixelShader"
			| "VertexShader"
	)
}

/// Scalar, vector, and matrix type names, such as `float`, `float3`, `uint16_t2`, or `float4x4`.
fn is_numeric_type(name: &str) -> bool {
	const SCALAR_TYPES: [&str; 21] = [
		"bool",
		"int",
		"uint",
		"dword",
		"half",
		"float",
		"double",
		"min16float",
		"min10float",
		"min16int",
		"min12int",
		"min16uint",
		"int16_t",
		"uint16_t",
		"int32_t",
		"uint32_t",
		"int64_t",
		"uint64_t",
		"float16_t",
		"float32_t",
		"float64_t",
	];

	/// Reports whether `suffix` is empty, a vector size `N`, or a matrix shape `RxC`, with every size in 1 to 4.
	fn is_shape_suffix(suffix: &str) -> bool {
		let is_size = |byte: &u8| (b'1'..=b'4').contains(byte);
		match suffix.as_bytes() {
			[] => true,
			[size] => is_size(size),
			[rows, b'x', columns] => is_size(rows) && is_size(columns),
			_ => false,
		}
	}

	SCALAR_TYPES
		.iter()
		.any(|scalar| name.strip_prefix(scalar).is_some_and(is_shape_suffix))
}

/// Resource, sampler, stream, and ray-tracing object type names.
fn is_object_type(name: &str) -> bool {
	matches!(
		name,
		"Buffer" | "RWBuffer" | "ByteAddressBuffer" | "RWByteAddressBuffer" | "StructuredBuffer"
			| "RWStructuredBuffer" | "AppendStructuredBuffer" | "ConsumeStructuredBuffer" | "ConstantBuffer"
			| "TextureBuffer" | "RasterizerOrderedBuffer" | "RasterizerOrderedByteAddressBuffer"
			| "RasterizerOrderedStructuredBuffer" | "texture" | "Texture" | "Texture1D" | "Texture1DArray"
			| "Texture2D" | "Texture2DArray" | "Texture2DMS" | "Texture2DMSArray" | "Texture3D" | "TextureCube"
			| "TextureCubeArray" | "RWTexture1D" | "RWTexture1DArray" | "RWTexture2D" | "RWTexture2DArray"
			| "RWTexture3D" | "RasterizerOrderedTexture1D" | "RasterizerOrderedTexture1DArray"
			| "RasterizerOrderedTexture2D" | "RasterizerOrderedTexture2DArray" | "RasterizerOrderedTexture3D"
			| "FeedbackTexture2D" | "FeedbackTexture2DArray" | "sampler" | "sampler1D" | "sampler2D" | "sampler3D"
			| "samplerCUBE" | "sampler_state" | "SamplerState" | "SamplerComparisonState" | "InputPatch"
			| "OutputPatch" | "PointStream" | "LineStream" | "TriangleStream" | "RaytracingAccelerationStructure"
			| "RayDesc" | "RayQuery" | "BuiltInTriangleIntersectionAttributes"
	)
}

/// Intrinsic functions and resource methods that the HLSL backend calls when it lowers BESL intrinsics, and the
/// common intrinsics that raw HLSL blocks may call.
///
/// A BESL local with one of these names would shadow the function at the call site.
fn is_builtin_function(name: &str) -> bool {
	matches!(
		name,
		"abs" | "acos" | "all" | "any" | "asfloat" | "asin" | "asint" | "asuint" | "atan" | "atan2" | "ceil" | "clamp"
			| "clip" | "cos" | "cosh" | "countbits" | "cross" | "ddx" | "ddy" | "degrees" | "determinant" | "distance"
			| "dot" | "exp" | "exp2" | "f16tof32" | "f32tof16" | "faceforward" | "firstbithigh" | "firstbitlow"
			| "floor" | "fma" | "fmod" | "frac" | "fwidth" | "isfinite" | "isinf" | "isnan" | "isnormal" | "ldexp"
			| "length" | "lerp" | "log" | "log10" | "log2" | "mad" | "max" | "min" | "modf" | "mul" | "normalize"
			| "pow" | "radians" | "rcp" | "reflect" | "refract" | "reversebits" | "round" | "rsqrt" | "saturate"
			| "sign" | "sin" | "sincos" | "sinh" | "smoothstep" | "sqrt" | "step" | "tan" | "tanh" | "transpose"
			| "trunc" | "InterlockedAdd" | "InterlockedAnd" | "InterlockedCompareExchange"
			| "InterlockedCompareStore" | "InterlockedExchange" | "InterlockedMax" | "InterlockedMin"
			| "InterlockedOr" | "InterlockedXor" | "GroupMemoryBarrier" | "GroupMemoryBarrierWithGroupSync"
			| "AllMemoryBarrier" | "AllMemoryBarrierWithGroupSync" | "DeviceMemoryBarrier"
			| "DeviceMemoryBarrierWithGroupSync" | "WaveGetLaneIndex" | "WaveGetLaneCount" | "WaveActiveBallot"
			| "WaveReadLaneAt" | "WaveReadLaneFirst" | "WaveActiveAnyTrue" | "WaveActiveAllTrue"
			| "WaveActiveCountBits" | "WaveIsFirstLane" | "SetMeshOutputCounts" | "DispatchMesh" | "NonUniformResourceIndex"
			| "Sample" | "SampleLevel" | "SampleGrad" | "Load" | "GetDimensions"
	)
}

/// Names the HLSL backend declares next to user code.
///
/// `main` is reserved on purpose: escaping turns the BESL entry point into `besl_main`, the HLSL entry point name
/// that pipelines load. Names such as `_besl_interface_position` are absent on purpose: the BESL front end
/// declares them, and the engine looks them up by their unescaped name.
fn is_backend_name(name: &str) -> bool {
	matches!(
		name,
		"main"
			| "PI" | "PushConstant"
			| "ObjectPayload"
			| "VertexOutput"
			| "PrimitiveOutput"
			| "payload"
			| "render_target_array_index"
			| "dispatch_thread_id"
			| "group_thread_id"
			| "group_id"
			| "group_thread_index"
			| "_besl_image_size"
			| "_besl_subgroup_ballot_any"
			| "_besl_subgroup_ballot_find_lsb"
			| "_besl_subgroup_ballot_count"
			| "_besl_subgroup_ballot_and_not"
			| "_besl_fma_f16"
	)
}
