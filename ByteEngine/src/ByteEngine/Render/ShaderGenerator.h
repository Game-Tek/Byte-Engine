#pragma once

#include <GTSL/String.hpp>
#include "ByteEngine/Application/AllocatorReferences.h"

struct Node {
	enum class Type : uint8 {
		VARIABLE, FUNCTION, SHADER_RESULT, OPERATOR, LITERAL
	} ValueType;

	Node() : ValueType(Type::SHADER_RESULT) {}
	Node(const GTSL::StaticString<32> name) : ValueType(Type::FUNCTION), Name(name) {
		if(std::islower(name[0])) {
			ValueType = Type::OPERATOR;
		}
	}

	Node(const GTSL::StaticString<32> type, const GTSL::StaticString<32> name) : ValueType(Type::VARIABLE), Name(name), Type(type) {
		if(std::isdigit(name[0])) {
			ValueType = Type::LITERAL;
		}
	}

	void AddInput(const Node& input) { Inputs.EmplaceBack(&input); }

	GTSL::StaticString<32> Name, Type;

	auto GetName() const {
		return Name;
	}

	struct Connection {
		const Node* Other;
	};
	GTSL::StaticVector<Connection, 8> Inputs;
};

struct Shader {
	Shader(const GTSL::ShortString<32> name, const GAL::ShaderType type) : Name(name), Type(type) {}

	void AddInput(const Node& node) { Inputs.EmplaceBack(&node); }
	void AddLayer(const char8_t* string) {
		Layers.EmplaceBack(string);
	}

	void AddVertexElement(Pipeline::VertexElement vertex_element) {
		VertexElements.EmplaceBack(vertex_element);
	}

	GTSL::ShortString<32> Name;
	GAL::ShaderType Type;
	GTSL::StaticVector<const Node*, 8> Inputs;
	GTSL::StaticVector<GTSL::ShortString<32>, 8> Layers;

	//vertex
	GTSL::StaticVector<GAL::Pipeline::VertexElement, 32> VertexElements;
};

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
		string += u8"vec3 barycenter(vec2 coords) { return vec3(1.0f - coords.x - coords.y, coords.x, coords.y); }\n";
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
		default:;
		}


		string += u8' '; string += name; string += u8";\n";
	};

	for (uint8 i = 0; i < vertexElements.ElementCount(); ++i) {
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

inline GTSL::StaticString<8192> GenerateShader(Shader& shader) {
	GTSL::StaticString<8192> string;
	AddExtensions(string, shader.Type);
	AddDataTypesAndDescriptors(string, shader.Type);

	struct ShaderFunction {
		struct FunctionSignature {
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 8> Parameters;
			GTSL::StaticString<32> ReturnType;
		};

		GTSL::StaticVector<FunctionSignature, 8> FunctionVersions;
	};

	GTSL::HashMap<Id, ShaderFunction, GTSL::DefaultAllocatorReference> functions(16, 1.0f);

	auto declFunc = [&](const GTSL::StaticString<32>& ret, const GTSL::StaticString<32>& name, GTSL::Range<const GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>*> parameters, const GTSL::StaticString<128>& impl) {
		auto functionByName = functions.TryEmplace(Id(name));

		if(functionByName) {
		}

		auto& functionVersion = functionByName.Get().FunctionVersions.EmplaceBack(); //TODO: check if function sig. exists
		functionVersion.ReturnType = ret;
		functionVersion.Parameters.PushBack(parameters);

		string += ret; string += u8' ';  string += name;

		string += u8"(";

		uint32 paramCount = parameters.ElementCount();

		for(uint32 i = 0; i < paramCount; ++i) {
			string += parameters[i].First; //type
			string += u8' ';
			string += parameters[i].Second; //name

			if (i != paramCount - 1) { string += u8", "; }
		}

		string += u8")\n{\n	";
		string += impl;
		string += u8"\n}\n";
	};

	// LEAF  LEAF  LEAF
	//   \   /  \   /
	//    MID    MID
	//	    \   /
	//       TOP

	// Process leaves first, and emit code

	auto evalFunc = [&](const Node* node, uint32 offset, uint32* paramCount, auto&& self) -> GTSL::StaticString<32> {
		switch (node->ValueType) {
		case Node::Type::FUNCTION: {
			auto& functionCollection = functions[Id(node->Name)];

			uint32 l = 0;

			for (; l < functionCollection.FunctionVersions.GetLength(); ++l) {
				uint32 p = offset;

				for (; p < functionCollection.FunctionVersions[l].Parameters.GetLength(); ++p) {
					if (functionCollection.FunctionVersions[l].Parameters[p].First != self(node->Inputs[p].Other, p, nullptr, self)) {
						break;
					}
				}

				if (p == functionCollection.FunctionVersions[l].Parameters.GetLength()) { break; }
			}

			if (l == functionCollection.FunctionVersions.GetLength()) { BE_ASSERT(false, u8"No compatible override found!"); }

			if(paramCount)
				*paramCount = functionCollection.FunctionVersions[l].Parameters.GetLength();

			return functionCollection.FunctionVersions[l].ReturnType;
		}
		}
	};

	auto pipi = [&](const Node* node, auto&& self) -> GTSL::StaticString<32> {
		switch (node->ValueType) {
		case Node::Type::VARIABLE: {
			string += node->Type; string += u8' '; string += node->Name; string += u8" = ";

			for (auto& e : node->Inputs) {
				self(e.Other, self);
			}

			string += u8";\n";

			return node->Type;
		}
		case Node::Type::FUNCTION: {			
			uint32 t = 1;
			uint32 paramCount = 0;

			auto retType = evalFunc(node, 0, &paramCount, evalFunc);

			if (paramCount) {
				t = (node->Inputs.GetLength() / paramCount) + 1;
			} else {
				string += node->Name; string += u8"()";
				return retType;
			}

			for(uint32 i = 0; i < t; ++i) {
				string += node->Name; string += u8"(";
			}

			for (uint32 i = 0, pg = 0; auto & e : node->Inputs) {
				self(e.Other, self);

				if((i % (paramCount - 1) == 0 && i)) { string += u8")"; }

				if (i != node->Inputs.GetLength() - 1) { string += u8", "; }

				++i; pg = i / paramCount;
			}

			return retType;
		}
		case Node::Type::OPERATOR: {
			uint32 i = 0;
			for (auto& e : node->Inputs) {
				self(e.Other, self);
				if (i != node->Inputs.GetLength() - 1) { string += u8" * "; }
				++i;
			}

			return {};
		}
		case Node::Type::LITERAL: {
			string += node->Type;
			string += u8'(';
			string += node->Name;
			string += u8')';
			break;
		}
		case Node::Type::SHADER_RESULT: {
			switch (shader.Type) {
			case GAL::ShaderType::VERTEX: {
				string += u8"gl_Position = ";

				for (auto& e : node->Inputs) {
					self(e.Other, self);
				}

				string += u8";\n";

				return {};
			}
			case GAL::ShaderType::FRAGMENT: {
				for (auto& e : node->Inputs) {
					self(e.Other, self);
				}

				string += u8"out_Color = ";

				string += node->Inputs[0].Other->Name;

				string += u8";\n";

				return {};
			}
			}
			break;
		}
		}

		return {};
	};

	string += u8R"(layout(buffer_reference, scalar, buffer_reference_align = 4) buffer CameraProperties
{
	mat4 view;
	mat4 proj;
	mat4 viewInverse;
	mat4 projInverse;
};

)";

	{
		switch (shader.Type) {
		case GAL::ShaderType::VERTEX: {
			shader.AddLayer(u8"InstanceData");

			string += u8R"(struct StaticMesh
{
	mat4 ModelMatrix;
	ptr_t VertexBuffer;
	ptr_t IndexBuffer;
	uint MaterialInstance;
};

layout(buffer_reference, scalar, buffer_reference_align = 4) buffer StaticMeshRenderGroupData
{
	StaticMesh Meshes[];
};

layout(location = 0) out localVertexShaderOut
{
	vec3 position;
	vec3 normal;
} out_LocalVertex;

layout(location = 4) out viewSpaceVertexShaderOut
{
	vec3 position;
	vec3 normal;
} out_ViewSpaceVertex;

layout(location = 8) out worldSpaceVertexShaderOut
{
	vec3 position;
} out_WorldSpaceVertex;
)";

			AddVertexShaderLayout(string, shader.VertexElements);

			break;
		}
		case GAL::ShaderType::FRAGMENT: {
			shader.AddLayer(u8"InstanceData");

			string += u8R"(struct StaticMesh
{
	mat4 ModelMatrix;
	ptr_t VertexBuffer;
	ptr_t IndexBuffer;
	uint MaterialInstance;
};

layout(buffer_reference, scalar, buffer_reference_align = 4) buffer StaticMeshRenderGroupData
{
	StaticMesh Meshes[];
};

layout(buffer_reference, scalar, buffer_reference_align = 4) buffer MaterialData
{
	TextureReference Albedo[];
};

layout(location = 0) out vec4 out_Color;
layout(location = 1) out vec3 out_Position;
layout(location = 2) out vec3 out_Normal;

layout(location = 0) in localVertexShaderOut
{
	vec3 position;
	vec3 normal;
} in_LocalVertex;

layout(location = 4) in viewSpaceVertexShaderOut
{
	vec3 position;
	vec3 normal;
} in_ViewSpaceVertex;

layout(location = 8) in worldSpaceVertexShaderOut
{
	vec3 position;
} in_WorldSpaceVertex;
)";
			break;
		}
		}
	}

	{ //push constant
		string += u8"layout(push_constant, scalar) uniform Data\n{\n";

		for (auto& l : shader.Layers) {
			string += u8'	'; string += u8"ptr_t "; string += l; string += u8";\n";
		}

		string += u8"} invocationInfo;\n";
	}

	switch (shader.Type) {
	case GAL::ShaderType::VERTEX: {
		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"mat4", u8"GetInstancePosition", parameters, u8"return StaticMeshRenderGroupData(invocationInfo.InstanceData).Meshes[0].ModelMatrix;");
		} {
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"mat4", u8"GetCameraViewMatrix", parameters, u8"return CameraProperties(invocationInfo.CameraData).view;");
		} {
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"mat4", u8"GetCameraProjectionMatrix", parameters, u8"return CameraProperties(invocationInfo.CameraData).proj;");
		} {
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"vec4", u8"GetVertexPosition", parameters, u8"return vec4(in_Position, 1.0);");
		}

		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{ { u8"float", u8"cosTheta" }, { u8"vec3", u8"F0"} };
			declFunc(u8"vec3", u8"fresnelSchlick", parameters, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
		}

		break;
	}
	case GAL::ShaderType::MESH:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::INTERSECTION: {
		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"mat4", u8"GetInstancePosition", parameters, u8"return StaticMeshRenderGroupData(invocationInfo.InstanceData).Meshes[0].ModelMatrix;");
		} {
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"mat4", u8"GetCameraViewMatrix", parameters, u8"return CameraProperties(invocationInfo.CameraData).view;");
		} {
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"mat4", u8"GetCameraProjectionMatrix", parameters, u8"return CameraProperties(invocationInfo.CameraData).proj;");
		} {
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"vec4", u8"GetVertexPosition", parameters, u8"return vec4(in_Position, 1.0);");
		}

		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{ { u8"float", u8"cosTheta" }, { u8"vec3", u8"F0"} };
			declFunc(u8"vec3", u8"fresnelSchlick", parameters, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
		}

		break;
	}

	case GAL::ShaderType::TESSELLATION_CONTROL: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION: break;
	case GAL::ShaderType::GEOMETRY: break;

	case GAL::ShaderType::FRAGMENT: {
		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{ { u8"float", u8"cosTheta" }, { u8"vec3", u8"F0"} };
			declFunc(u8"vec3", u8"fresnelSchlick", parameters, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
		}

		break;
	}

	case GAL::ShaderType::COMPUTE: break;
	case GAL::ShaderType::TASK: break;
	case GAL::ShaderType::RAY_GEN: break;
	case GAL::ShaderType::MISS: break;
	case GAL::ShaderType::CALLABLE: break;
	}

	{
		GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{ { u8"vec2", u8"coords" } };
		declFunc(u8"vec3", u8"barycenter", parameters, u8"return vec3(1.0f - coords.x - coords.y, coords.x, coords.y);");
	}

	{ //main
		string += u8"void main()\n{\n";

		Node vertexShaderResult;
		vertexShaderResult.AddInput(*shader.Inputs[0]);

		pipi(&vertexShaderResult, pipi);

		string += u8"}";
	}

	return string;
}