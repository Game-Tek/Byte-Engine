//! GLSL names that a BESL identifier must not take after lowering.
//!
//! The GLSL [`Generator`](super::analysis::Generator) consults [`is_reserved`] through
//! [`NodeEmitter::is_reserved_identifier`](crate::shader::generator::NodeEmitter::is_reserved_identifier) and prefixes
//! matching names with [`RESERVED_IDENTIFIER_PREFIX`](crate::shader::generator::RESERVED_IDENTIFIER_PREFIX).

/// Reports whether `name` collides with a GLSL keyword, reserved word, built-in type or function, or a name the
/// GLSL backend declares itself.
pub(super) fn is_reserved(name: &str) -> bool {
	// The `gl_` prefix is reserved for built-in variables in every GLSL scope.
	name.starts_with("gl_") || is_keyword(name) || is_builtin_function(name) || is_backend_name(name)
}

/// Keywords, reserved words, and built-in type names from GLSL 4.60, GL_KHR_vulkan_glsl, and the extensions the
/// backend enables.
fn is_keyword(name: &str) -> bool {
	matches!(
		name,
		// Language keywords.
		"attribute" | "const" | "uniform" | "varying" | "buffer" | "shared" | "coherent" | "volatile" | "restrict"
			| "readonly" | "writeonly" | "atomic_uint" | "layout" | "centroid" | "flat" | "smooth" | "noperspective"
			| "patch" | "sample" | "invariant" | "precise" | "break" | "continue" | "do" | "for" | "while" | "switch"
			| "case" | "default" | "if" | "else" | "subroutine" | "in" | "out" | "inout" | "true" | "false" | "discard"
			| "return" | "struct" | "lowp" | "mediump" | "highp" | "precision"
			// Scalar, vector, and matrix types.
			| "void" | "bool" | "int" | "uint" | "float" | "double"
			| "vec2" | "vec3" | "vec4" | "ivec2" | "ivec3" | "ivec4" | "bvec2" | "bvec3" | "bvec4"
			| "uvec2" | "uvec3" | "uvec4" | "dvec2" | "dvec3" | "dvec4"
			| "mat2" | "mat3" | "mat4" | "mat2x2" | "mat2x3" | "mat2x4" | "mat3x2" | "mat3x3" | "mat3x4"
			| "mat4x2" | "mat4x3" | "mat4x4" | "dmat2" | "dmat3" | "dmat4" | "dmat2x2" | "dmat2x3" | "dmat2x4"
			| "dmat3x2" | "dmat3x3" | "dmat3x4" | "dmat4x2" | "dmat4x3" | "dmat4x4"
			// Opaque types.
			| "sampler" | "samplerShadow" | "sampler1D" | "sampler2D" | "sampler3D" | "samplerCube"
			| "sampler1DShadow" | "sampler2DShadow" | "samplerCubeShadow" | "sampler1DArray" | "sampler2DArray"
			| "sampler1DArrayShadow" | "sampler2DArrayShadow" | "isampler1D" | "isampler2D" | "isampler3D"
			| "isamplerCube" | "isampler1DArray" | "isampler2DArray" | "usampler1D" | "usampler2D" | "usampler3D"
			| "usamplerCube" | "usampler1DArray" | "usampler2DArray" | "sampler2DRect" | "sampler2DRectShadow"
			| "isampler2DRect" | "usampler2DRect" | "samplerBuffer" | "isamplerBuffer" | "usamplerBuffer"
			| "sampler2DMS" | "isampler2DMS" | "usampler2DMS" | "sampler2DMSArray" | "isampler2DMSArray"
			| "usampler2DMSArray" | "samplerCubeArray" | "samplerCubeArrayShadow" | "isamplerCubeArray"
			| "usamplerCubeArray" | "image1D" | "iimage1D" | "uimage1D" | "image2D" | "iimage2D" | "uimage2D"
			| "image3D" | "iimage3D" | "uimage3D" | "image2DRect" | "iimage2DRect" | "uimage2DRect" | "imageCube"
			| "iimageCube" | "uimageCube" | "imageBuffer" | "iimageBuffer" | "uimageBuffer" | "image1DArray"
			| "iimage1DArray" | "uimage1DArray" | "image2DArray" | "iimage2DArray" | "uimage2DArray"
			| "imageCubeArray" | "iimageCubeArray" | "uimageCubeArray" | "image2DMS" | "iimage2DMS" | "uimage2DMS"
			| "image2DMSArray" | "iimage2DMSArray" | "uimage2DMSArray" | "texture1D" | "texture2D" | "texture3D"
			| "textureCube" | "texture1DArray" | "texture2DArray" | "textureCubeArray" | "textureBuffer"
			| "texture2DMS" | "texture2DMSArray" | "texture2DRect" | "itexture2D" | "utexture2D" | "subpassInput"
			| "isubpassInput" | "usubpassInput" | "subpassInputMS" | "isubpassInputMS" | "usubpassInputMS"
			// Reserved for future use.
			| "common" | "partition" | "active" | "asm" | "class" | "union" | "enum" | "typedef" | "template" | "this"
			| "resource" | "goto" | "inline" | "noinline" | "public" | "static" | "extern" | "external" | "interface"
			| "long" | "short" | "half" | "fixed" | "unsigned" | "superp" | "input" | "output" | "hvec2" | "hvec3"
			| "hvec4" | "fvec2" | "fvec3" | "fvec4" | "sampler3DRect" | "filter" | "sizeof" | "cast" | "namespace"
			| "using"
			// Explicit arithmetic types and qualifiers from the enabled extensions.
			| "int8_t" | "uint8_t" | "int16_t" | "uint16_t" | "int32_t" | "uint32_t" | "int64_t" | "uint64_t"
			| "float16_t" | "float32_t" | "float64_t" | "i8vec2" | "i8vec3" | "i8vec4" | "u8vec2" | "u8vec3"
			| "u8vec4" | "i16vec2" | "i16vec3" | "i16vec4" | "u16vec2" | "u16vec3" | "u16vec4" | "i32vec2"
			| "i32vec3" | "i32vec4" | "u32vec2" | "u32vec3" | "u32vec4" | "i64vec2" | "i64vec3" | "i64vec4"
			| "u64vec2" | "u64vec3" | "u64vec4" | "f16vec2" | "f16vec3" | "f16vec4" | "f32vec2" | "f32vec3"
			| "f32vec4" | "f64vec2" | "f64vec3" | "f64vec4" | "f16mat2" | "f16mat3" | "f16mat4" | "f32mat2"
			| "f32mat3" | "f32mat4" | "perprimitiveEXT" | "perviewEXT" | "taskPayloadSharedEXT" | "nonuniformEXT"
	)
}

/// Built-in functions the GLSL backend calls when it lowers BESL intrinsics.
///
/// A BESL local with one of these names would shadow the function at the call site.
fn is_builtin_function(name: &str) -> bool {
	matches!(
		name,
		"abs" | "acos" | "all" | "any" | "asin" | "atan" | "ceil" | "clamp" | "cos" | "cross" | "degrees" | "distance"
			| "dot" | "exp" | "exp2" | "faceforward" | "findLSB" | "findMSB" | "floor" | "fma" | "fract" | "fwidth"
			| "inversesqrt" | "isinf" | "isnan" | "length" | "log" | "log2" | "max" | "min" | "mix" | "mod"
			| "normalize" | "notEqual" | "pow" | "radians" | "reflect" | "refract" | "round" | "sign" | "sin"
			| "smoothstep" | "sqrt" | "step" | "tan" | "transpose" | "inverse" | "texture" | "textureLod"
			| "textureGrad" | "textureSize" | "texelFetch" | "imageLoad" | "imageStore" | "imageSize" | "imageAtomicOr"
			| "atomicAdd" | "atomicAnd" | "atomicCompSwap" | "atomicExchange" | "atomicMax" | "atomicMin" | "atomicOr"
			| "atomicXor" | "barrier" | "subgroupBallot" | "subgroupBallotBitCount" | "subgroupBallotFindLSB"
			| "subgroupBroadcast" | "SetMeshOutputsEXT"
	)
}

/// Global names the GLSL backend declares next to user code.
///
/// `main` is absent on purpose: the BESL entry point must keep that name because it is the GLSL entry point.
fn is_backend_name(name: &str) -> bool {
	matches!(
		name,
		"PI" | "PushConstant" | "push_constant" | "_besl_is_finite" | "_besl_is_normal"
	)
}
