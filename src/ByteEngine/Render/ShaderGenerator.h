#pragma once

#include <GTSL/String.hpp>
#include "ByteEngine/Application/AllocatorReferences.h"
#include <GAL/RenderCore.h>
#include <locale>
#include <GTSL/HashMap.hpp>
#include <ByteEngine/Id.h>
#include <GTSL/Vector.hpp>
#include <GAL/Pipelines.h>

using StructElement = GTSL::Pair<GTSL::ShortString<32>, GTSL::ShortString<32>>;

struct Node {
	enum class Type : uint8 {
		VARIABLE, VAR_DEC, FUNCTION, SHADER_RESULT, OPERATOR, LITERAL, SHADER_PARAMETER
	} ValueType;

	Node() : ValueType(Type::SHADER_RESULT) {}

	Node(const Type type, const GTSL::StringView name) : ValueType(type), Name(name) {}

	Node(const GTSL::StringView name) : ValueType(Type::FUNCTION), Name(name) {
		if(name[0] == u8'=' or name[0] == u8'*') {
			ValueType = Type::OPERATOR;
		}
	}

	Node(const GTSL::StringView type, const GTSL::StringView name) : Name(name), Type(type) {
		if(std::isdigit(name[0])) {
			ValueType = Type::LITERAL;
		} else {
			ValueType = Type::VAR_DEC;
		}
	}

	GTSL::ShortString<32> Name, Type;

	auto GetName() const -> GTSL::StringView {
		return Name;
	}

	struct Connection {
		const Node* Other;
	};
	GTSL::StaticVector<Connection, 8> Inputs;

	void AddInput(const Node& input) { Inputs.EmplaceBack(&input); }
};

struct Shader {
	enum class Class { VERTEX, PIXEL, COMPUTE };

	Shader(const GTSL::ShortString<32> name, const Class clss) : Name(name), Class(clss) {}

	void AddTexture(const GTSL::ShortString<32> element) { Textures.EmplaceBack(element); }

	void AddInput(const Node& node) { Inputs.EmplaceBack(&node); }
	void AddLayer(const StructElement element) {
		Layers.EmplaceBack(element);
	}

	void AddOutput(const StructElement element) {
		Outputs.EmplaceBack(element);
	}

	void AddVertexElement(GAL::Pipeline::VertexElement vertex_element) {
		VertexElements.EmplaceBack(vertex_element);
	}

	void SetThreadSize(const GTSL::Extent3D size) { threadSize = size; }

	void RemoveInput() {
		Inputs.PopBack();
	}

	GTSL::ShortString<32> Name;
	Class Class;
	GTSL::StaticVector<const Node*, 8> Inputs;
	GTSL::StaticVector<StructElement, 8> Layers;
	GAL::ShaderType TargetSemantics;

	GTSL::StaticVector<GTSL::ShortString<32>, 8> Textures;
	GTSL::StaticVector<StructElement, 8> Outputs;

	//vertex
	GTSL::StaticVector<GAL::Pipeline::VertexElement, 32> VertexElements;

	//compute
	GTSL::Extent3D threadSize;
};

template<typename T>
void AddDefaults(GTSL::String<T>& string, GAL::ShaderType shaderType) {
	string += u8"#version 460 core\n"; //push version
	
	switch (shaderType) {
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
	string += u8"layout(row_major) uniform; layout(row_major) buffer;\n"; //matrix order definitions
}

template<typename T>
void AddDataTypesAndDescriptors(GTSL::String<T>& string, GAL::ShaderType shaderType) {	
	string += u8"layout(set = 0, binding = 0) uniform sampler2D textures[];\n"; //textures descriptor
	string += u8"layout(set = 0, binding = 1) uniform image2D images[];\n"; //textures descriptor	
	string += u8"#define ptr_t uint64_t\n";
	string += u8"struct TextureReference { uint Instance; };\n"; //basic datatypes
}

inline GTSL::StaticString<8192> GenerateShader(Shader& shader) {
	GTSL::StaticString<8192> string;
	AddDefaults(string, shader.TargetSemantics);
	AddDataTypesAndDescriptors(string, shader.TargetSemantics);

	struct ShaderFunction {
		struct FunctionSignature {
			GTSL::StaticVector<StructElement, 8> Parameters;
			GTSL::ShortString<32> ReturnType;
		};

		GTSL::StaticVector<FunctionSignature, 8> FunctionVersions;
	};

	GTSL::HashMap<Id, ShaderFunction, GTSL::DefaultAllocatorReference> functions(16, 1.0f);
	GTSL::HashMap<Id, GTSL::StaticVector<StructElement, 32>, GTSL::DefaultAllocatorReference> structs(16, 1.0f);

	auto declFunc = [&](const GTSL::StringView ret, const GTSL::StringView name, GTSL::Range<const StructElement*> parameters, const GTSL::StringView impl) {
		auto functionByName = functions.TryEmplace(Id(name));
		BE_ASSERT(functionByName, u8"Already exists");
		auto& functionVersion = functionByName.Get().FunctionVersions.EmplaceBack(); //TODO: check if function sig. exists
		functionVersion.ReturnType = ret;
		functionVersion.Parameters.PushBack(parameters);

		string += ret; string += u8' ';  string += name;

		string += u8"(";

		uint32 paramCount = parameters.ElementCount();

		for(uint32 i = 0; i < paramCount; ++i) {
			string += parameters[i].First; string += u8' '; string += parameters[i].Second;
			if (i != paramCount - 1) { string += u8", "; }
		}

		string += u8") { ";
		string += impl;
		string += u8" }\n";
	};

	auto declStruct = [&](GTSL::ShortString<32> ne, GTSL::Range<const StructElement*> structElements, bool ref, bool readOnly = true) {
		GTSL::StaticString<32> name(ne);

		if (ref)
			name += u8"Pointer";

		auto& st = structs.Emplace(Id(name));

		if (!structElements.ElementCount()) { return; }

		if (ref) {
			string += u8"layout(buffer_reference, scalar, buffer_reference_align = 4) ";

			if (readOnly)
				string += u8"readonly ";

			string += u8"buffer ";
		} else {
			string += u8"struct ";
		}

		string += name; string += u8" { ";

		for (auto& e : structElements) {
			string += e.First; string += u8' '; string += e.Second; string += u8"; ";
			st.EmplaceBack(e);
		}

		string += u8"};\n";
	};

	//global data
	declStruct(u8"globalData", GTSL::StaticVector<StructElement, 32>{ { u8"uint", u8"dummy" }}, true);

	if (shader.Textures.GetLength()) {
		GTSL::StaticVector<StructElement, 32> strEl;

		for (auto& e : shader.Textures) {
			strEl.EmplaceBack(u8"TextureReference", e);
		}

		declStruct(u8"shaderParameters", strEl, true);

		shader.AddLayer({ u8"shaderParameters", u8"shader_parameters" });
	}

	auto evalNode = [&](const Node* node, uint32 offset, uint32* paramCount, auto&& self) -> GTSL::ShortString<32> {
	};

	// LEAF  LEAF  LEAF
	//   \   /  \   /
	//    MID    MID
	//	    \   /
	//       TOP
	// Process leaves first, and emit code
	auto placeNode = [&](const Node* node, auto&& self) -> GTSL::ShortString<32> {
		switch (node->ValueType) {
		case Node::Type::VARIABLE: {
			string += node->Name;
			for (auto& e : node->Inputs) { self(e.Other, self); }
			return node->Type;
		}
		case Node::Type::VAR_DEC: {
			string += node->Type; string += u8' '; string += node->Name;
			for (auto& e : node->Inputs) { self(e.Other, self); }
			break;
		}
		case Node::Type::FUNCTION: {			
			//auto& functionCollection = functions[Id(node->Name)]; uint32 l = 0;
			//
			//for (; l < functionCollection.FunctionVersions.GetLength(); ++l) {
			//	for (; p < functionCollection.FunctionVersions[l].Parameters.GetLength(); ++p) {
			//		if (functionCollection.FunctionVersions[l].Parameters[p].First != self(node->Inputs[p].Other, p, nullptr, self)) { //todo: fix, will access node's inputs, when none are available
			//			break;
			//		}
			//	}
			//
			//	if (p == functionCollection.FunctionVersions[l].Parameters.GetLength()) { break; }
			//}
			//if (l == functionCollection.FunctionVersions.GetLength()) { BE_ASSERT(false, u8"No compatible override found!"); }
			//for(uint32 i = 0; i < t; ++i) {
			//	string += node->Name; string += u8"("; }
			//for (uint32 i = 0, pg = 0; auto & e : node->Inputs) {
			//	self(e.Other, self);
			//	if((i % (paramCount - 1) == 0 && i)) { string += u8")"; }
			//	if (i != node->Inputs.GetLength() - 1) { string += u8", "; }
			//	++i; pg = i / paramCount;
			//}
			
			string += node->Name; string += u8"(";

			for (uint32 i = 0; auto & e : node->Inputs) {
				self(e.Other, self);
				if (i != node->Inputs.GetLength() - 1) { string += u8", "; }
				++i;
			}

			string += u8")";

			break;
		}
		case Node::Type::OPERATOR: {
			uint32 i = 0;
			for (auto& e : node->Inputs) {
				self(e.Other, self);
				if (i != node->Inputs.GetLength() - 1) {
					string += u8' '; string += node->Name.begin()[0]; string += u8' ';
				}
				++i;
			}

			break;
		}
		case Node::Type::LITERAL: {
			string += node->Type; string += u8'('; string += node->Name; string += u8')';
			break;
		}
		case Node::Type::SHADER_RESULT: {
			switch (shader.TargetSemantics) {
			case GAL::ShaderType::VERTEX: {
				string += u8"gl_Position = ";
				break;
			}
			case GAL::ShaderType::FRAGMENT: {
				string += u8"out_Color = ";
				break;
			}
			case GAL::ShaderType::CLOSEST_HIT: {
				string += u8"payload = ";
				break;
			}
			}

			for (auto& e : node->Inputs) { self(e.Other, self); }

			break;
		}
		case Node::Type::SHADER_PARAMETER: {
			string += u8"invocationInfo.shader_parameters."; string += node->Name;
			break;
		}
		}

		return {};
	};

	{
		GTSL::StaticVector<StructElement, 32> vertexElements;
		vertexElements.EmplaceBack(u8"mat4", u8"view");
		vertexElements.EmplaceBack(u8"mat4", u8"proj");
		vertexElements.EmplaceBack(u8"mat4", u8"viewInverse");
		vertexElements.EmplaceBack(u8"mat4", u8"projInverse");
		declStruct(u8"camera", vertexElements, true);
	}

	auto genVertexStruct = [&]() {
		GTSL::StaticVector<StructElement, 32> vertexElements;

		for (auto& e : shader.VertexElements) {
			GTSL::ShortString<32> dataType;

			switch (e.Type) {
			case GAL::ShaderDataType::FLOAT:  dataType = GTSL::Range(u8"float"); break;
			case GAL::ShaderDataType::FLOAT2: dataType = GTSL::Range(u8"vec2"); break;
			case GAL::ShaderDataType::FLOAT3: dataType = GTSL::Range(u8"vec3"); break;
			case GAL::ShaderDataType::FLOAT4: dataType = GTSL::Range(u8"vec4"); break;
			case GAL::ShaderDataType::UINT16: dataType = GTSL::Range(u8"uint16_t"); break;
			case GAL::ShaderDataType::UINT32: dataType = GTSL::Range(u8"uint"); break;
			case GAL::ShaderDataType::INT:    dataType = GTSL::Range(u8"int"); break;
			case GAL::ShaderDataType::INT2: break;
			case GAL::ShaderDataType::INT3: break;
			case GAL::ShaderDataType::INT4: break;
			case GAL::ShaderDataType::BOOL: break;
			case GAL::ShaderDataType::MAT3: break;
			case GAL::ShaderDataType::MAT4: break;
			default:;
			}

			vertexElements.EmplaceBack(dataType, e.Identifier);
		}

		declStruct(u8"vertex", vertexElements, false);
		declStruct(u8"vertex", vertexElements, true);
	};

	if (shader.Class == Shader::Class::PIXEL) {
		shader.VertexElements.EmplaceBack(GAL::Pipeline::VertexElement{ u8"dummy", GAL::ShaderDataType::UINT32 });
	}

	genVertexStruct();

	if (shader.Class != Shader::Class::COMPUTE) {
		{
			GTSL::StaticVector<StructElement, 32> elements;
			elements.EmplaceBack(u8"uint16_t", u8"i");
			declStruct(u8"index", elements, true);
		}
	}

	{ //
		GTSL::StaticVector<StructElement, 32> elements;
		elements.EmplaceBack(u8"uint", u8"dummy");
		declStruct(u8"renderPass", elements, true);
	}

	switch (shader.Class) {
	case Shader::Class::VERTEX: {
		GTSL::StaticVector<StructElement, 32> elements;
		elements.EmplaceBack(u8"mat4", u8"ModelMatrix");
		elements.EmplaceBack(u8"vertexPointer", u8"VertexBuffer");
		elements.EmplaceBack(u8"indexPointer", u8"IndexBuffer");
		elements.EmplaceBack(u8"uint", u8"MaterialInstance");
		declStruct(u8"instanceData", elements, true);
		shader.AddLayer({ u8"instanceData", u8"instance" });

		if (shader.TargetSemantics == GAL::ShaderType::VERTEX) {
			for (uint8 i = 0; i < shader.VertexElements.GetLength(); ++i) {
				const auto& att = shader.VertexElements[i];
				GTSL::StaticString<64> name(u8"in_"); name += att.Identifier;

				string += u8"layout(location = "; ToString(string, i); string += u8") in ";

				switch (att.Type) {
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
			}
		}

		break;
	}
	case Shader::Class::PIXEL: {
		GTSL::StaticVector<StructElement, 32> elements;
		elements.EmplaceBack(u8"mat4", u8"ModelMatrix");
		elements.EmplaceBack(u8"vertexPointer", u8"VertexBuffer");
		elements.EmplaceBack(u8"indexPointer", u8"IndexBuffer");
		elements.EmplaceBack(u8"uint", u8"MaterialInstance");
		declStruct(u8"instanceData", elements, true);
		shader.AddLayer({ u8"instanceData", u8"instance" });

		if (shader.TargetSemantics == GAL::ShaderType::FRAGMENT) {
			for (uint8 i = 0; i < shader.Outputs.GetLength(); ++i) {
				string += u8"layout(location ="; ToString(string, i); string += u8") out ";
				string += shader.Outputs[i].First; string += u8" out_"; string += shader.Outputs[i].Second; string += u8";\n";
			}
		}

		break;
	}
	}

	if (shader.TargetSemantics == GAL::ShaderType::RAY_GEN) {
		shader.AddLayer({ u8"rayDispatch", u8"ray_dispatch_data" });
	}

	{ //push constant
		string += u8"layout(push_constant, scalar) uniform _invocationInfo { ";
		for (auto& l : shader.Layers) { string += l.First; string += u8"Pointer"; string += u8' '; string += l.Second; string += u8"; "; }
		string += u8"} invocationInfo;\n";
	}

	switch (shader.Class) {
	case Shader::Class::VERTEX: {		
		declFunc(GTSL::Range(u8"mat4"), GTSL::Range(u8"GetInstancePosition"), {}, GTSL::Range(u8"return invocationInfo.instance.ModelMatrix;"));
		declFunc(GTSL::Range(u8"mat4"), GTSL::Range(u8"GetCameraViewMatrix"), {}, GTSL::Range(u8"return invocationInfo.camera.view;"));
		declFunc(GTSL::Range(u8"mat4"), GTSL::Range(u8"GetCameraProjectionMatrix"), {}, GTSL::Range(u8"return invocationInfo.camera.proj;"));
		declFunc(GTSL::Range(u8"vec4"), GTSL::Range(u8"GetVertexPosition"), {}, GTSL::Range(u8"return vec4(in_POSITION, 1.0);"));
		
		break;
	}
	case Shader::Class::PIXEL: {
		declFunc(GTSL::Range(u8"mat4"), GTSL::Range(u8"GetInstancePosition"), {}, GTSL::Range(u8"return invocationInfo.instance.ModelMatrix;"));
		declFunc(GTSL::Range(u8"mat4"), GTSL::Range(u8"GetCameraViewMatrix"), {}, GTSL::Range(u8"return invocationInfo.camera.view;"));
		declFunc(GTSL::Range(u8"mat4"), GTSL::Range(u8"GetCameraProjectionMatrix"), {}, GTSL::Range(u8"return invocationInfo.camera.proj;"));
		//declFunc(u8"vec4", u8"GetVertexPosition", {}, u8"return vec4(in_POSITION, 1.0);");

		break;
	}
	case Shader::Class::COMPUTE: break;
	default: ;
	}

	switch (shader.TargetSemantics) {
	case GAL::ShaderType::VERTEX: {
		string += u8"layout(location = 0) out vertexData { vec2 texture_coordinates; } vertexOut;\n";
		break;
	}
	case GAL::ShaderType::MESH:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::INTERSECTION: {
		GTSL::StaticVector<StructElement, 2> parameters{};
		declFunc(GTSL::Range(u8"vec4"), GTSL::Range(u8"GetVertexPosition"), parameters, GTSL::Range(u8"return vec4(in_Position, 1.0);"));

		{
			GTSL::StaticVector<StructElement, 32> elements;
			elements.EmplaceBack(u8"ptr_t", u8"MaterialData"); elements.EmplaceBack(u8"ptr_t", u8"InstanceData");
			declStruct(u8"shaderEntry", elements, false);
		}

		string += u8"layout(shaderRecordEXT, scalar) buffer shader { shaderEntry shaderEntries[]; };\n";

		{
			GTSL::StaticVector<StructElement, 32> elements;
			elements.EmplaceBack(u8"uint16_t", u8"i");
			declStruct(u8"index", elements, true);
		}
		
		string += u8"hitAttributeEXT vec2 hitBarycenter;\n";
		string += u8"layout(location = 0) rayPayloadInEXT vec4 payload;\n";

		break;
	}
	case GAL::ShaderType::TESSELLATION_CONTROL: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION: break;
	case GAL::ShaderType::GEOMETRY: break;
	case GAL::ShaderType::FRAGMENT: {
		string += u8"layout(location = 0) in vertexData { vec2 texture_coordinates; } vertexIn;\n";

		declFunc(GTSL::Range(u8"vec2"), GTSL::Range(u8"GetFragmentPosition"), {}, GTSL::Range(u8"return gl_FragCoord.xy;"));
		declFunc(GTSL::Range(u8"vec2"), GTSL::Range(u8"GetVertexTextureCoordinates"), {}, GTSL::Range(u8"return vertexIn.texture_coordinates;"));

		break;
	}
	case GAL::ShaderType::COMPUTE: break;
	case GAL::ShaderType::TASK: break;
	case GAL::ShaderType::RAY_GEN: {
		string += u8"layout(location = 0) rayPayloadEXT vec4 payload;\n";

		{
			GTSL::StaticVector<StructElement, 32> elements;
			for (uint8 i = 0; i < shader.Outputs.GetLength(); ++i) { elements.EmplaceBack(shader.Outputs[i]); }
			declStruct(u8"_renderPass", elements, true);
		}
		
		{
			GTSL::StaticVector<StructElement, 32> elements;
			elements.EmplaceBack(u8"uint64_t", u8"AccelerationStructure");
			elements.EmplaceBack(u8"uint", u8"RayFlags");
			elements.EmplaceBack(u8"uint", u8"SBTRecordOffset"); elements.EmplaceBack(u8"uint", u8"SBTRecordStride"); elements.EmplaceBack(u8"uint", u8"MissIndex"); elements.EmplaceBack(u8"uint", u8"Payload");
			elements.EmplaceBack(u8"float", u8"tMin");
			elements.EmplaceBack(u8"float", u8"tMax");
			declStruct(u8"_rayTrace", elements, true);
		}

		declFunc(GTSL::Range(u8"vec2"), GTSL::Range(u8"GetFragmentPosition"), {}, GTSL::Range(u8"const vec2 pixelCenter = vec2(gl_LaunchIDEXT.xy) + vec2(0.5f);\nreturn pixelCenter / vec2(gl_LaunchSizeEXT.xy);"));

		{
			GTSL::StaticVector<StructElement, 2> parameters{ { u8"vec3", u8"origin" }, { u8"vec3", u8"direction" } };
			declFunc(GTSL::Range(u8"void"), GTSL::Range(u8"TraceRay"), parameters, GTSL::Range(u8"_rayTrace r = _rayTrace(invocationInfo.RayDispatchData);\ntraceRayEXT(accelerationStructureEXT(r.AccelerationStructure), r.RayFlags, 0xff, r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, origin, r.tMin, direction, r.tMax, r.Payload);"));
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
		GTSL::StaticVector<StructElement, 2> parameters{ { u8"vec2", u8"coords" } };
		declFunc(GTSL::Range(u8"vec3"), GTSL::Range(u8"Barycenter"), parameters, GTSL::Range(u8"return vec3(1.0f - coords.x - coords.y, coords.x, coords.y);"));
	} {
		GTSL::StaticVector<StructElement, 2> parameters{ { u8"TextureReference", u8"textureReference" }, { u8"vec2", u8"texCoord" } };
		declFunc(GTSL::Range(u8"vec4"), GTSL::Range(u8"Sample"), parameters, GTSL::Range(u8"return texture(textures[nonuniformEXT(textureReference.Instance)], texCoord);"));
	} {
		GTSL::StaticVector<StructElement, 2> parameters{ { u8"float", u8"cosTheta" }, { u8"vec3", u8"F0"} };
		declFunc(GTSL::Range(u8"vec3"), GTSL::Range(u8"FresnelSchlick"), parameters, GTSL::Range(u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);"));
	}
	
	//{
	//	GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 3> parameters{ { u8"vec2", u8"texture_coordinate" }, { u8"float", u8"depth_from_depth_buffer" }, { u8"mat4", u8"inverse_projection_matrix" } };
	//	declFunc(u8"vec3", u8"WorldPosFromDepth", parameters, u8"vec3 clip_space_position = vec3(texture_coordinate, depth_from_depth_buffer) * 2.0 - vec3(1.0);\nvec4 view_position = vec4(vec2(inverse_projection_matrix[0][0], inverse_projection_matrix[1][1]) * clip_space_position.xy, inverse_projection_matrix[2][3] * clip_space_position.z + inverse_projection_matrix[3][3]);\nreturn (view_position.xyz / view_position.w);\n");
	//}
	
	{
		GTSL::StaticVector<StructElement, 2> parameters{ { u8"vec3", u8"a" } };
		declFunc(u8"vec3", u8"Normalize", parameters, u8"return normalize(a);");
	}

	{ //main
		switch (shader.TargetSemantics) {
		case GAL::ShaderType::COMPUTE: {
			GTSL::Extent3D size = shader.threadSize;
			string += u8"layout(local_size_x="; ToString(string, size.Height); string += u8", local_size_y="; ToString(string, size.Width); string += u8", local_size_z="; ToString(string, size.Depth); string += u8") in;\n";
			break;
		}
		case GAL::ShaderType::MESH: {
			string += u8"layout(local_size_x="; ToString(string, 32); string += u8") in;\n";
			string += u8"layout(triangles) out;\n";
			string += u8"layout(max_vertices=64, max_primitives=126) out;\n";
			break;
		}
		}

		string += u8"void main() {\n";

		switch (shader.TargetSemantics) {
		case GAL::ShaderType::VERTEX: {
			string += u8"vertexOut.texture_coordinates = in_TEXTURE_COORDINATES;\n";
			break;
		}
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
			string += u8"StaticMeshPointer instance = StaticMeshPointer(shaderEntries[gl_InstanceCustomIndexEXT]);\n";
			string += u8"uint indeces[3] = uint[3](instance.IndexBuffer[3 * gl_PrimitiveID + 0], instance.IndexBuffer[3 * gl_PrimitiveID + 1], instance.IndexBuffer[3 * gl_PrimitiveID + 2]);\n";
			string += u8"vertex vertices[3] = vertex[](instance.VertexBuffer[indeces[0]], instance.VertexBuffer[indeces[1]], instance.VertexBuffer[indeces[2]]);\n";
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

		for (uint32 i = 0; i < shader.Inputs.GetLength(); ++i) {
			placeNode(shader.Inputs[i], placeNode); string += u8";\n";
		}

		string += u8"}";
	}

	return string;
}