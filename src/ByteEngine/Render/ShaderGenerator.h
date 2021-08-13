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

	Node(const GTSL::StaticString<32> type, const GTSL::StaticString<32> name) : Name(name), Type(type) {
		if(std::isdigit(name[0])) {
			ValueType = Type::LITERAL;
		} else if(std::isupper(type[0])) {
			ValueType = Type::FUNCTION;
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
	enum class Class {
		VERTEX, PIXEL, COMPUTE
	};

	Shader(const GTSL::ShortString<32> name, const Class clss) : Name(name), Class(clss) {}

	void AddInput(const Node& node) { Inputs.EmplaceBack(&node); }
	void AddLayer(const char8_t* string) {
		Layers.EmplaceBack(string);
	}

	void AddVertexElement(Pipeline::VertexElement vertex_element) {
		VertexElements.EmplaceBack(vertex_element);
	}

	void RemoveInput() {
		Inputs.PopBack();
	}

	GTSL::ShortString<32> Name;
	Class Class;
	GTSL::StaticVector<const Node*, 8> Inputs;
	GTSL::StaticVector<GTSL::ShortString<32>, 8> Layers;
	GAL::ShaderType TargetSemantics;

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
	string += u8"#extension GL_EXT_buffer_reference2 : enable\n";
	string += u8"#extension GL_EXT_shader_image_load_formatted : enable\n";
}

template<typename T>
void AddDataTypesAndDescriptors(GTSL::String<T>& string, GAL::ShaderType shaderType) {
	string += u8"layout(row_major) uniform; layout(row_major) buffer;\n"; //matrix order definitions
	
	string += u8"layout(set = 0, binding = 0) uniform sampler2D textures[];\n"; //textures descriptor
	string += u8"layout(set = 0, binding = 1) uniform image2D images[];\n"; //textures descriptor
	
	string += u8"#define ptr_t uint64_t\n";
	string += u8"struct TextureReference { uint Instance; };\n"; //basic datatypes
}

template<typename T>
inline auto AddVertexShaderLayout(GTSL::String<T>& string, const GTSL::Range<const GAL::Pipeline::VertexElement*> vertexElements)
{
	auto addElement = [&](GTSL::ShortString<64> name, uint16 index, GAL::ShaderDataType type) {
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
		default: {
			GTSL::ShortString<64> name(u8"in_"); name += att.Identifier;
			addElement(name, i, att.Type);
			break;
		}
		}
	}
}

inline GTSL::StaticString<8192> GenerateShader(Shader& shader) {
	GTSL::StaticString<8192> string;
	AddExtensions(string, shader.TargetSemantics);
	AddDataTypesAndDescriptors(string, shader.TargetSemantics);

	struct ShaderFunction {
		struct FunctionSignature {
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 8> Parameters;
			GTSL::StaticString<32> ReturnType;
		};

		GTSL::StaticVector<FunctionSignature, 8> FunctionVersions;
	};

	GTSL::HashMap<Id, ShaderFunction, GTSL::DefaultAllocatorReference> functions(16, 1.0f);

	auto declFunc = [&](const GTSL::StaticString<32>& ret, const GTSL::StaticString<32>& name, GTSL::Range<const GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>*> parameters, const GTSL::StaticString<512>& impl) {
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

		string += u8") {\n	";
		string += impl;
		string += u8" }\n";
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
			switch (shader.TargetSemantics) {
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
			case GAL::ShaderType::CLOSEST_HIT: {
				for (auto& e : node->Inputs) {
					self(e.Other, self);
				}

				string += u8"payload = ";

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

	string += u8"layout(buffer_reference, scalar, buffer_reference_align = 4) buffer CameraProperties { mat4 view; mat4 proj; mat4 viewInverse; mat4 projInverse; };\n";

	{
		auto genVertexStruct = [&]() {
			string += u8"struct VERTEX { ";

			for (auto& e : shader.VertexElements) {
				switch (e.Type) {
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

				string += u8" "; string += e.Identifier; string += u8"; ";
			}

			string += u8" };\n";
		};

		switch (shader.Class) {
		case Shader::Class::VERTEX: {
			string += u8"layout(buffer_reference, scalar, buffer_reference_align = 4) buffer StaticMeshRenderGroupData { mat4 ModelMatrix; ptr_t VertexBuffer; ptr_t IndexBuffer; uint MaterialInstance; };\n";
			break;
		}
		case Shader::Class::PIXEL: break;
		case Shader::Class::COMPUTE: break;
		default: ;
		}

		switch (shader.TargetSemantics) {
		case GAL::ShaderType::VERTEX: {
			shader.AddLayer(u8"InstanceData");

			string += u8R"(layout(location = 0) out localVertexShaderOut { vec3 position; vec3 normal; } out_LocalVertex;
layout(location = 4) out viewSpaceVertexShaderOut {	vec3 position; vec3 normal; } out_ViewSpaceVertex;
layout(location = 8) out worldSpaceVertexShaderOut { vec3 position; } out_WorldSpaceVertex;
)";

			AddVertexShaderLayout(string, shader.VertexElements);

			break;
		}
		case GAL::ShaderType::FRAGMENT: {
			shader.AddLayer(u8"InstanceData");

			string += u8R"(layout(buffer_reference, scalar, buffer_reference_align = 4) buffer MaterialData { TextureReference Albedo; };
layout(location = 0) out vec4 out_Color;
layout(location = 1) out vec3 out_Position;
layout(location = 2) out vec3 out_Normal;

layout(location = 0) in localVertexShaderOut { vec3 position; vec3 normal; } in_LocalVertex;
layout(location = 4) in viewSpaceVertexShaderOut { vec3 position; vec3 normal; } in_ViewSpaceVertex;
layout(location = 8) in worldSpaceVertexShaderOut { vec3 position; } in_WorldSpaceVertex; 
)";
			break;
		}
		case GAL::ShaderType::RAY_GEN: {
			shader.AddLayer(u8"RayDispatchData");
			break;
		}
		case GAL::ShaderType::CLOSEST_HIT: {
			shader.AddLayer(u8"RayDispatchData");
			genVertexStruct();

			break;
		}
		case GAL::ShaderType::MESH: {
			genVertexStruct();
			break;
		}
		}
	}

	{ //push constant
		string += u8"layout(push_constant, scalar) uniform Data {\n";

		for (auto& l : shader.Layers) {
			string += u8'	'; string += u8"ptr_t "; string += l; string += u8";\n";
		}

		string += u8"} invocationInfo;\n";
	}

	switch (shader.TargetSemantics) {
	case GAL::ShaderType::VERTEX: {
		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"mat4", u8"GetInstancePosition", parameters, u8"return StaticMeshRenderGroupData(invocationInfo.InstanceData)[0].ModelMatrix;");
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

		break;
	}
	case GAL::ShaderType::MESH:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::INTERSECTION: {
		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"mat4", u8"GetInstancePosition", parameters, u8"return StaticMeshRenderGroupData(invocationInfo.InstanceData)[0].ModelMatrix;");
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

		string += u8"struct ShaderEntry { ptr_t MaterialData; ptr_t BufferReference InstanceData; };\n";
		string += u8"layout(shaderRecordEXT, scalar) buffer ShaderDataBuffer { ShaderEntry ShaderEntries[]; };\n";
		string += u8"layout(buffer_reference, scalar, buffer_reference_align = 4) readonly buffer StaticMeshPointer { mat4 ModelMatrix; ptr_t VertexBuffer, IndexBuffer; uint MaterialInstance; };\n";
		string += u8"layout(buffer_reference, scalar, buffer_reference_align = 4) readonly buffer MaterialDataBuffer { TextureReference Albedo; };\n";
		string += u8"layout(buffer_reference, scalar, buffer_reference_align = 4) readonly buffer Vertices { VERTEX v[]; };\n";
		string += u8"layout(buffer_reference, scalar, buffer_reference_align = 2) readonly buffer Indices { uint16_t i[]; };\n";
		string += u8"hitAttributeEXT vec2 hitBarycenter;\n";
		string += u8"layout(location = 0) rayPayloadInEXT vec4 payload;\n";

		break;
	}
	case GAL::ShaderType::TESSELLATION_CONTROL: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION: break;
	case GAL::ShaderType::GEOMETRY: break;
	case GAL::ShaderType::FRAGMENT: {
		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"vec2", u8"GetFragmentPosition", parameters, u8"return gl_FragCoord.xy;");
		}

		break;
	}
	case GAL::ShaderType::COMPUTE: break;
	case GAL::ShaderType::TASK: break;
	case GAL::ShaderType::RAY_GEN: {
		string += u8"layout(location = 0) rayPayloadEXT vec4 payload;\n";
		string += u8"layout(buffer_reference, scalar, buffer_reference_align = 4) buffer RenderPass { TextureReference Albedo; };\n";
		string += u8"layout(buffer_reference, scalar, buffer_reference_align = 4) buffer RayTrace { uint64_t AccelerationStructure; uint RayFlags, SBTRecordOffset, SBTRecordStride, MissIndex, Payload; float tMin, tMax; };\n";

		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{};
			declFunc(u8"vec2", u8"GetFragmentPosition", parameters, u8"const vec2 pixelCenter = vec2(gl_LaunchIDEXT.xy) + vec2(0.5f);\nreturn pixelCenter / vec2(gl_LaunchSizeEXT.xy);");
		}

		{
			GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{ { u8"vec3", u8"origin" }, { u8"vec3", u8"direction" } };
			declFunc(u8"void", u8"TraceRay", parameters, u8"RayTrace r = RayTrace(invocationInfo.RayDispatchData);\ntraceRayEXT(accelerationStructureEXT(r.AccelerationStructure), r.RayFlags, 0xff, r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, origin, r.tMin, direction, r.tMax, r.Payload);");
		}

		break;
	}
	case GAL::ShaderType::MISS: {
		string += u8"layout(location = 0) rayPayloadInEXT vec4 payload;\n";
		break;
	}
	case GAL::ShaderType::CALLABLE: break;
	}

	{
		GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{ { u8"vec2", u8"coords" } };
		declFunc(u8"vec3", u8"Barycenter", parameters, u8"return vec3(1.0f - coords.x - coords.y, coords.x, coords.y);");
	} {
		GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{ { u8"TextureReference", u8"textureReference" }, { u8"vec2", u8"texCoord" } };
		declFunc(u8"vec4", u8"SampleTexture", parameters, u8"return texture(textures[nonuniformEXT(textureReference.Instance)], texCoord);");
	}

	{
		GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 2> parameters{ { u8"float", u8"cosTheta" }, { u8"vec3", u8"F0"} };
		declFunc(u8"vec3", u8"FresnelSchlick", parameters, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
	}
	//{
	//	GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 3> parameters{ { u8"vec2", u8"texture_coordinate" }, { u8"float", u8"depth_from_depth_buffer" }, { u8"mat4", u8"inverse_projection_matrix" } };
	//	declFunc(u8"vec3", u8"WorldPosFromDepth", parameters, u8"vec3 clip_space_position = vec3(texture_coordinate, depth_from_depth_buffer) * 2.0 - vec3(1.0);\nvec4 view_position = vec4(vec2(inverse_projection_matrix[0][0], inverse_projection_matrix[1][1]) * clip_space_position.xy, inverse_projection_matrix[2][3] * clip_space_position.z + inverse_projection_matrix[3][3]);\nreturn (view_position.xyz / view_position.w);\n");
	//}

	{
		GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 3> parameters{ { u8"vec3", u8"a" } };
		declFunc(u8"vec3", u8"Normalize", parameters, u8"return normalize(a);");
	}

	{ //main
		string += u8"void main() {\n";

		switch (shader.TargetSemantics) {
		case GAL::ShaderType::VERTEX: break;
		case GAL::ShaderType::FRAGMENT: break;
		case GAL::ShaderType::COMPUTE: break;
		case GAL::ShaderType::TASK: break;
		case GAL::ShaderType::MESH: break;
		case GAL::ShaderType::RAY_GEN: {
			string += u8"payload = vec4(1.0f, 0.0f, 0.0f, 1.0f);\n";
			break;
		}
		case GAL::ShaderType::ANY_HIT: break;
		case GAL::ShaderType::CLOSEST_HIT: {
			string += u8"payload = vec4(1.0f, 0.0f, 0.0f, 1.0f);\n";
			string += u8"StaticMeshPointer instance = StaticMeshPointer(ShaderEntries[gl_InstanceCustomIndexEXT]);\n";
			string += u8"uint indeces[3] = uint[3](Indices(instance.IndexBuffer).i[3 * gl_PrimitiveID + 0], Indices(instance.IndexBuffer).i[3 * gl_PrimitiveID + 1], Indices(instance.IndexBuffer).i[3 * gl_PrimitiveID + 2]);\n";
			string += u8"VERTEX vertices[3] = VERTEX[](Vertices(instance.VertexBuffer).v[indeces[0]], Vertices(instance.VertexBuffer).v[indeces[1]], Vertices(instance.VertexBuffer).v[indeces[2]]);\n";
			string += u8"const vec3 barycenter = Barycenter(hitBarycenter);\n";
			string += u8"vec3 normal = vertices[0].Normal * barycenter.x + vertices[1].Normal * barycenter.y + vertices[2].Normal * barycenter.z;\n";
			string += u8"vec2 texCoord = vertices[0].TexCoords * barycenter.x + vertices[1].TexCoords * barycenter.y + vertices[2].TexCoords * barycenter.z;\n";
			break;
		}
		case GAL::ShaderType::MISS: {
			string += u8"payload = vec4(0.0f, 0.0f, 0.0f, 1.0f);\n";
			break;
		}
		case GAL::ShaderType::INTERSECTION: break;
		case GAL::ShaderType::CALLABLE: break;
		default: ;
		}

		if(shader.Inputs.GetLength())
			pipi(shader.Inputs[0], pipi);

		string += u8"}";
	}

	return string;
}