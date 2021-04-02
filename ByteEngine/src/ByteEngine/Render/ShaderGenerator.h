#pragma once
#include <GTSL/String.hpp>

#include "ByteEngine/Application/AllocatorReferences.h"

template<typename T>
inline void AddExtensions(GTSL::String<T>& string, GAL::ShaderType shaderType)
{
	string += "#version 460 core\n"; //push version
	
	switch (shaderType)
	{
	case GAL::ShaderType::VERTEX_SHADER: break;
	case GAL::ShaderType::TESSELLATION_CONTROL_SHADER: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION_SHADER: break;
	case GAL::ShaderType::GEOMETRY_SHADER: break;
	case GAL::ShaderType::FRAGMENT_SHADER: break;
	case GAL::ShaderType::COMPUTE_SHADER: break;
		
	case GAL::ShaderType::RAY_GEN:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::MISS:
	case GAL::ShaderType::INTERSECTION:
	case GAL::ShaderType::CALLABLE:
		string += "#extension GL_EXT_ray_tracing : enable\n"; break;
	default: ;
	}
	
	string += "#extension GL_EXT_shader_16bit_storage : enable\n";
	string += "#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable\n";
	string += "#extension GL_EXT_nonuniform_qualifier : enable\n";
	string += "#extension GL_EXT_scalar_block_layout : enable\n";
	string += "#extension GL_EXT_buffer_reference : enable\n";
	string += "#extension GL_EXT_shader_image_load_formatted : enable\n";
}

template<typename T>
inline void AddDataTypesAndDescriptors(GTSL::String<T>& string, GAL::ShaderType shaderType)
{
	switch (shaderType)
	{
	case GAL::ShaderType::VERTEX_SHADER: break;
	case GAL::ShaderType::TESSELLATION_CONTROL_SHADER: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION_SHADER: break;
	case GAL::ShaderType::GEOMETRY_SHADER: break;
	case GAL::ShaderType::FRAGMENT_SHADER: break;
	case GAL::ShaderType::COMPUTE_SHADER: break;

	case GAL::ShaderType::RAY_GEN:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::MISS:
	case GAL::ShaderType::INTERSECTION:
	case GAL::ShaderType::CALLABLE:
		break;
	default:;
	}

	string += "layout(row_major) uniform; layout(row_major) buffer;\n"; //matrix order definitions
	
	string += "layout(set = 0, binding = 0) uniform sampler2D textures[];\n"; //textures descriptor
	
	string += "struct BufferReference { uint Address; }; struct TextureReference { uint Instance; };\n"; //basic datatypes
	string += "uint64_t BRP(BufferReference reference) { return uint64_t(reference.Address * 16); }\n";
}

template<typename T>
inline auto GenerateShader(GTSL::String<T>& string, GAL::ShaderType shaderType)
{
	AddExtensions(string, shaderType);
	AddDataTypesAndDescriptors(string, shaderType);
}
