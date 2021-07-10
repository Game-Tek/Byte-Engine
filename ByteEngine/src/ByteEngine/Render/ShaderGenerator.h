#pragma once

#include <GTSL/String.hpp>
#include "ByteEngine/Application/AllocatorReferences.h"

template<typename T>
void AddExtensions(GTSL::String<T>& string, GAL::ShaderType shaderType)
{
	string += u8"#version 460 core\n"; //push version
	
	switch (shaderType)
	{
	case GAL::ShaderType::VERTEX: break;
	case GAL::ShaderType::TESSELLATION_CONTROL: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION: break;
	case GAL::ShaderType::GEOMETRY: break;
	case GAL::ShaderType::FRAGMENT: break;
	case GAL::ShaderType::COMPUTE: break;
		
	case GAL::ShaderType::RAY_GEN:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::MISS:
	case GAL::ShaderType::INTERSECTION:
	case GAL::ShaderType::CALLABLE:
		string += u8"#extension GL_EXT_ray_tracing : enable\n";
		break;
	default: ;
	}
	
	string += u8"#extension GL_EXT_shader_16bit_storage : enable\n";
	string += u8"#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable\n";
	string += u8"#extension GL_EXT_nonuniform_qualifier : enable\n";
	string += u8"#extension GL_EXT_scalar_block_layout : enable\n";
	string += u8"#extension GL_EXT_buffer_reference : enable\n";
	string += u8"#extension GL_EXT_shader_image_load_formatted : enable\n";
}

template<typename T>
void AddDataTypesAndDescriptors(GTSL::String<T>& string, GAL::ShaderType shaderType) {
	switch (shaderType)
	{
	case GAL::ShaderType::VERTEX: break;
	case GAL::ShaderType::TESSELLATION_CONTROL: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION: break;
	case GAL::ShaderType::GEOMETRY: break;
	case GAL::ShaderType::FRAGMENT: break;
	case GAL::ShaderType::COMPUTE: break;

	case GAL::ShaderType::RAY_GEN:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::MISS:
	case GAL::ShaderType::INTERSECTION:
	case GAL::ShaderType::CALLABLE:
		break;
	default:;
	}

	string += u8"layout(row_major) uniform; layout(row_major) buffer;\n"; //matrix order definitions
	
	string += u8"layout(set = 0, binding = 0) uniform sampler2D textures[];\n"; //textures descriptor
	
	string += u8"#define ptr_t uint64_t\n";
	string += u8"struct TextureReference { uint Instance; };\n"; //basic datatypes
}

template<typename T>
void AddCommonFunctions(GTSL::String<T>& string, GAL::ShaderType shaderType) {
	switch (shaderType)
	{
	case GAL::ShaderType::VERTEX: break;
	case GAL::ShaderType::TESSELLATION_CONTROL: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION: break;
	case GAL::ShaderType::GEOMETRY: break;
	case GAL::ShaderType::COMPUTE: break;
	case GAL::ShaderType::RAY_GEN: break;
	case GAL::ShaderType::MISS: break;
	case GAL::ShaderType::CALLABLE: break;

	case GAL::ShaderType::FRAGMENT:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::INTERSECTION:
		string += u8"vec3 fresnelSchlick(float cosTheta, vec3 F0) { return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0); }\n";
		break;
	default:;
	}
}

template<typename T>
auto GenerateShader(GTSL::String<T>& string, GAL::ShaderType shaderType)
{
	AddExtensions(string, shaderType);
	AddDataTypesAndDescriptors(string, shaderType);
	AddCommonFunctions(string, shaderType);
}

//layout(location = 0) in vec3 in_Position;

template<typename T>
inline auto AddVertexShaderLayout(GTSL::String<T>& string, const GTSL::Range<const GAL::Pipeline::VertexElement*> vertexElements)
{
	auto addElement = [&](GTSL::ShortString<32> name, uint16 index, GAL::ShaderDataType type) {
		string += u8"layout(location = "; ToString(index, string); string += u8") in ";

		switch (type) {
		case GAL::ShaderDataType::FLOAT:  string += u8"float"; break;
		case GAL::ShaderDataType::FLOAT2: string += u8"vec2"; break;
		case GAL::ShaderDataType::FLOAT3: string += u8"vec3"; break;
		case GAL::ShaderDataType::FLOAT4: string += u8"vec4"; break;
		case GAL::ShaderDataType::INT: break;
		case GAL::ShaderDataType::INT2: break;
		case GAL::ShaderDataType::INT3: break;
		case GAL::ShaderDataType::INT4: break;
		case GAL::ShaderDataType::BOOL: break;
		case GAL::ShaderDataType::MAT3: break;
		case GAL::ShaderDataType::MAT4: break;
		default: ;
		}

		
		string += u8' '; string += name; string += u8";\n";
	};

	for(uint8 i = 0; i < vertexElements.ElementCount(); ++i) {
		const auto& att = vertexElements[i];
		
		switch (GTSL::Id64(att.Identifier)()) {
		case Hash(GAL::Pipeline::POSITION): addElement(u8"in_Position", i, att.Type); break;
		case Hash(GAL::Pipeline::NORMAL): addElement(u8"in_Normal", i, att.Type); break;
		case Hash(GAL::Pipeline::TANGENT): addElement(u8"in_Tangent", i, att.Type); break;
		case Hash(GAL::Pipeline::BITANGENT): addElement(u8"in_BiTangent", i, att.Type); break;
		case Hash(GAL::Pipeline::TEXTURE_COORDINATES): addElement(u8"in_TextureCoordinates", i, att.Type); break;
		}
	}
}
