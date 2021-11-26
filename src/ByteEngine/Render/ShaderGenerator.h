#pragma once

#include <GTSL/String.hpp>
#include "ByteEngine/Application/AllocatorReferences.h"
#include <GAL/RenderCore.h>
//#include <locale>
#include <GTSL/HashMap.hpp>
#include <ByteEngine/Id.h>
#include <GTSL/Vector.hpp>
#include <GAL/Pipelines.h>

#include <GTSL/JSON.hpp>

#include <GTSL/Tree.hpp>

//Object types are always stored ass the interface types, not the end target's name
struct StructElement {
	GTSL::ShortString<32> Type, Name, DefaultValue;
};

struct ShaderNode {
	enum class Type : uint8 {
		VARIABLE, VAR_DEC, FUNCTION, SHADER_RESULT, OPERATOR, LITERAL, SHADER_PARAMETER
	} ValueType;

	GTSL::ShortString<32> Name, TypeName;

	auto GetName() const -> GTSL::StringView {
		return Name;
	}
};

struct Shader {
	enum class Class { VERTEX, FRAGMENT, COMPUTE };

	Shader(const GTSL::StringView name, const Class clss) : Name(name), Type(clss) {}

	void AddShaderParameter(const StructElement element) { ShaderParameters.EmplaceBack(element); }
	void AddLayer(const StructElement element) {
		Layers.EmplaceBack(element);
	}

	void AddVertexElement(GAL::Pipeline::VertexElement vertex_element) {
		VertexElements.EmplaceBack(vertex_element);
	}

	void SetThreadSize(const GTSL::Extent3D size) { threadSize = size; }

	GTSL::ShortString<32> Name;
	Class Type;
	GTSL::StaticVector<StructElement, 8> Layers;
	GAL::ShaderType TargetSemantics;

	GTSL::StaticVector<StructElement, 8> ShaderParameters;

	//vertex
	GTSL::StaticVector<GAL::Pipeline::VertexElement, 32> VertexElements;

	GAL::ShaderStage ShaderStage;

	//compute
	GTSL::Extent3D threadSize;

	GTSL::StaticVector<GTSL::Tree<ShaderNode, BE::PAR>, 8> statements;
};

struct GPipeline
{
	GTSL::StaticVector<GAL::Pipeline::VertexElement, 32> VertexElements;
	GTSL::StaticVector<GAL::ShaderDataType, 32> VertexFragmentInterface;

	GTSL::StaticVector<GTSL::StaticVector<GTSL::StaticString<64>, 8>, 8> descriptors;
	GTSL::StaticVector<StructElement, 8> parameters;
};

inline GTSL::StaticString<8192> GenerateShader(Shader& shader, const GPipeline& pipeline) {
	GTSL::StaticString<2048> headerBlock, structBlock, functionBlock, declarationBlock, mainBlock;

	headerBlock += u8"#version 460 core\n"; //push version

	switch (shader.TargetSemantics) {
	case GAL::ShaderType::RAY_GEN:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::MISS:
	case GAL::ShaderType::INTERSECTION:
	case GAL::ShaderType::CALLABLE:
		headerBlock += u8"#extension GL_EXT_ray_tracing : enable\n";
		break;
	default:;
	}

	headerBlock += u8"#extension GL_EXT_shader_16bit_storage : enable\n";
	headerBlock += u8"#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable\n";
	headerBlock += u8"#extension GL_EXT_nonuniform_qualifier : enable\n";
	headerBlock += u8"#extension GL_EXT_scalar_block_layout : enable\n";
	headerBlock += u8"#extension GL_EXT_buffer_reference : enable\n";
	headerBlock += u8"#extension GL_EXT_buffer_reference2 : enable\n";
	headerBlock += u8"#extension GL_EXT_shader_image_load_formatted : enable\n";
	headerBlock += u8"layout(row_major) uniform; layout(row_major) buffer;\n"; //matrix order definitions

	structBlock += u8"struct TextureReference { uint Instance; };\n"; //basic datatypes
	structBlock += u8"struct ImageReference { uint Instance; };\n"; //basic datatypes

	for (uint32 si = 0; si < pipeline.descriptors; ++si) {
		auto& s = pipeline.descriptors[si];

		for (uint32 bi = 0; bi < s; ++bi) {
			declarationBlock += u8"layout(set="; ToString(declarationBlock, si);
			declarationBlock += u8",binding="; ToString(declarationBlock, bi);
			declarationBlock += u8") ";
			declarationBlock += s[bi]; declarationBlock += u8";\n";
		}
	}

	struct ShaderFunction {
		struct FunctionSignature {
			GTSL::StaticVector<StructElement, 8> Parameters;
			GTSL::ShortString<32> ReturnType;
			GTSL::StaticString<256> Body;
			bool Used = false;
		};

		GTSL::StaticVector<FunctionSignature, 8> FunctionVersions;
	};

	GTSL::HashMap<Id, ShaderFunction, GTSL::DefaultAllocatorReference> functions(16, 1.0f);
	GTSL::HashMap<Id, GTSL::StaticVector<StructElement, 32>, GTSL::DefaultAllocatorReference> structs(16, 1.0f);

	auto resolveTypeName = [&](const GTSL::StringView name) {
		switch (Hash(name)) {
		case GTSL::Hash(u8"float32"): return GTSL::StringView(u8"float");
		case GTSL::Hash(u8"vec2f"): return GTSL::StringView(u8"vec2");
		case GTSL::Hash(u8"vec3f"): return GTSL::StringView(u8"vec3");
		case GTSL::Hash(u8"vec4f"): return GTSL::StringView(u8"vec4");
		case GTSL::Hash(u8"uint64"): return GTSL::StringView(u8"uint64_t");
		case GTSL::Hash(u8"uint32"): return GTSL::StringView(u8"uint");
		case GTSL::Hash(u8"uint16"): return GTSL::StringView(u8"uint16_t");
		case GTSL::Hash(u8"ptr_t"): return GTSL::StringView(u8"uint64_t");
		}

		return name;
	};

	auto declareFunction = [&](const GTSL::StringView ret, const GTSL::StringView name, GTSL::Range<const StructElement*> parameters, const GTSL::StringView impl) {
		auto functionByName = functions.TryEmplace(Id(name));

		if(!functionByName) {
			auto eq = true;

			for(uint32 f = 0; f < functionByName.Get().FunctionVersions.GetLength(); ++f) {
				bool et = true;

				if (parameters.ElementCount() == GTSL::Range<const StructElement*>(functionByName.Get().FunctionVersions[f].Parameters).ElementCount()) {
					for (uint64 i = 0; i < parameters.ElementCount(); ++i) {
						if (parameters[i].Type != functionByName.Get().FunctionVersions[f].Parameters[i].Type or parameters[i].Name != functionByName.Get().FunctionVersions[f].Parameters[i].Name) { et = false; break; }
					}
				} else {
					et = false;
				}

				BE_ASSERT(!et, u8"Already exists");
			}
		}

		auto& functionVersion = functionByName.Get().FunctionVersions.EmplaceBack();
		functionVersion.ReturnType = ret;

		for(auto& e : parameters) { functionVersion.Parameters.EmplaceBack(e); }

		functionVersion.Body = impl;
	};

	auto declStruct = [&](GTSL::ShortString<32> ne, GTSL::Range<const StructElement*> structElements, bool ref, bool readOnly = true) {
		GTSL::StaticString<32> name(ne);

		if (ref) {
			name += u8"Pointer";
		}

		auto& st = structs.Emplace(Id(name));

		if (!structElements.ElementCount()) {
			st.EmplaceBack(u8"uint32", u8"dummy");
		} else {
			for (auto& e : structElements) {
				st.EmplaceBack(e);
			}
		}

		if (ref) {
			structBlock += u8"layout(buffer_reference, scalar, buffer_reference_align = 4) ";

			if (readOnly)
				structBlock += u8"readonly ";

			structBlock += u8"buffer ";
		} else {
			structBlock += u8"struct ";
		}

		structBlock += name; structBlock += u8" { ";

		for (auto& e : st) {
			structBlock += resolveTypeName(e.Type); structBlock += u8' '; structBlock += e.Name; structBlock += u8"; ";
		}

		structBlock += u8"};\n";
	};

	auto useFunction = [&](const GTSL::StringView name) {
		auto& function = functions[name].FunctionVersions[0];

		if (!function.Used) {
			functionBlock += resolveTypeName(function.ReturnType); functionBlock += u8' ';  functionBlock += name;

			functionBlock += u8"(";

			uint32 paramCount = function.Parameters.GetLength();

			for (uint32 i = 0; i < paramCount; ++i) {
				functionBlock += resolveTypeName(function.Parameters[i].Type); functionBlock += u8' '; functionBlock += function.Parameters[i].Name;
				if (i != paramCount - 1) { functionBlock += u8", "; }
			}

			functionBlock += u8") { ";
			functionBlock += function.Body;
			functionBlock += u8" }\n";

			function.Used = true;
		}
	};

	//global data
	declStruct(u8"globalData", {}, true);

	declStruct(u8"shaderParameters", pipeline.parameters, true);
	shader.AddLayer({ u8"shaderParameters", u8"shader_parameters" });

	using TTT = decltype(shader.statements[0].begin());

	auto placeNode = [&](TTT nodeHandle, uint32_t level, auto&& self) -> void {
		auto* node = &nodeHandle.Get();

		switch (node->ValueType) {
		case ShaderNode::Type::VARIABLE: {
			mainBlock += node->Name;
			break;
		}
		case ShaderNode::Type::VAR_DEC: {
			mainBlock += resolveTypeName(node->TypeName); mainBlock += u8' '; mainBlock += node->Name;
			break;
		}
		case ShaderNode::Type::FUNCTION: {
			useFunction(node->Name);

			mainBlock += node->Name; mainBlock += u8"(";

			for (uint32 i = 0; auto e : nodeHandle) {
				self(e, level + 1, self);
				if(i < nodeHandle.GetLength() - 1) {
					mainBlock += u8", ";
				}

				++i;
			}

			mainBlock += u8")";

			break;
		}
		case ShaderNode::Type::OPERATOR: {
			for (uint32 i = 0; auto e : nodeHandle) {
				self(e, level + 1, self);
				if (i < nodeHandle.GetLength() - 1) {
					mainBlock += u8' '; mainBlock += node->Name; mainBlock += u8' ';
				}

				++i;
			}

			break;
		}
		case ShaderNode::Type::LITERAL: {
			mainBlock += resolveTypeName(node->TypeName);
			mainBlock += u8'(';

			for (uint32 i = 0; auto e : nodeHandle) {
				self(e, level + 1, self);
				if (i < nodeHandle.GetLength() - 1) {
					mainBlock += u8", ";
				}

				++i;
			}

			mainBlock += u8')';

			break;
		}
		case ShaderNode::Type::SHADER_RESULT: {
			switch (shader.TargetSemantics) {
			case GAL::ShaderType::VERTEX: {
				mainBlock += u8"gl_Position = ";
				break;
			}
			case GAL::ShaderType::FRAGMENT: {
				mainBlock += u8"out_Color = ";
				break;
			}
			case GAL::ShaderType::CLOSEST_HIT: {
				mainBlock += u8"payload = ";
				break;
			}
			}

			for (uint32 i = 0; auto e : nodeHandle) {
				self(e, level + 1, self);

				++i;
			}

			break;
		}
		case ShaderNode::Type::SHADER_PARAMETER: {
			mainBlock += u8"invocationInfo.shader_parameters."; mainBlock += node->Name;
			break;
		}
		}
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

	genVertexStruct();

	if (shader.Type != Shader::Class::COMPUTE) {
		GTSL::StaticVector<StructElement, 32> elements;
		elements.EmplaceBack(u8"uint16", u8"i");
		declStruct(u8"index", elements, true);
	}

	declStruct(u8"renderPass", {}, true);

	switch (shader.Type) {
	case Shader::Class::VERTEX: {
		GTSL::StaticVector<StructElement, 32> elements;
		elements.EmplaceBack(u8"mat4", u8"ModelMatrix");
		elements.EmplaceBack(u8"vertexPointer", u8"VertexBuffer");
		elements.EmplaceBack(u8"indexPointer", u8"IndexBuffer");
		elements.EmplaceBack(u8"uint32", u8"MaterialInstance");
		declStruct(u8"instanceData", elements, true);
		shader.AddLayer({ u8"instanceData", u8"instance" });

		if (shader.TargetSemantics == GAL::ShaderType::VERTEX) {
			for (uint8 i = 0; i < shader.VertexElements.GetLength(); ++i) {
				const auto& att = shader.VertexElements[i];
				GTSL::StaticString<64> name(u8"in_"); name += att.Identifier;

				declarationBlock += u8"layout(location="; ToString(declarationBlock, i); declarationBlock += u8") in ";

				switch (att.Type) {
				case GAL::ShaderDataType::FLOAT:  declarationBlock += u8"float"; break;
				case GAL::ShaderDataType::FLOAT2: declarationBlock += u8"vec2"; break;
				case GAL::ShaderDataType::FLOAT3: declarationBlock += u8"vec3"; break;
				case GAL::ShaderDataType::FLOAT4: declarationBlock += u8"vec4"; break;
				case GAL::ShaderDataType::INT: break;
				case GAL::ShaderDataType::INT2: break;
				case GAL::ShaderDataType::INT3: break;
				case GAL::ShaderDataType::INT4: break;
				case GAL::ShaderDataType::BOOL: break;
				case GAL::ShaderDataType::MAT3: break;
				case GAL::ShaderDataType::MAT4: break;
				default:;
				}

				declarationBlock += u8' '; declarationBlock += name; declarationBlock += u8";\n";
			}
		}

		break;
	}
	case Shader::Class::FRAGMENT: {
		GTSL::StaticVector<StructElement, 32> elements;
		elements.EmplaceBack(u8"mat4", u8"ModelMatrix");
		elements.EmplaceBack(u8"vertexPointer", u8"VertexBuffer");
		elements.EmplaceBack(u8"indexPointer", u8"IndexBuffer");
		elements.EmplaceBack(u8"uint32", u8"MaterialInstance");
		declStruct(u8"instanceData", elements, true);
		shader.AddLayer({ u8"instanceData", u8"instance" });

		if (shader.TargetSemantics == GAL::ShaderType::FRAGMENT) {
			declarationBlock += u8"layout(location="; ToString(declarationBlock, 0); declarationBlock += u8") out ";
			declarationBlock += resolveTypeName(u8"vec4f"); declarationBlock += u8" out_"; declarationBlock += u8"Color"; declarationBlock += u8";\n";
		}

		break;
	}
	}

	if (shader.TargetSemantics == GAL::ShaderType::RAY_GEN) {
		shader.AddLayer({ u8"rayDispatch", u8"ray_dispatch_data" });
	}

	{ //push constant
		declarationBlock += u8"layout(push_constant, scalar) uniform _invocationInfo { ";
		for (auto& l : shader.Layers) { declarationBlock += resolveTypeName(l.Type); declarationBlock += u8"Pointer"; declarationBlock += u8' '; declarationBlock += l.Name; declarationBlock += u8"; "; }
		declarationBlock += u8"} invocationInfo;\n";
	}

	switch (shader.Type) {
	case Shader::Class::VERTEX: {		
		declareFunction(u8"mat4", u8"GetInstancePosition", {}, u8"return invocationInfo.instance.ModelMatrix;");
		declareFunction(u8"mat4", u8"GetCameraViewMatrix", {}, u8"return invocationInfo.camera.view;");
		declareFunction(u8"mat4", u8"GetCameraProjectionMatrix", {}, u8"return invocationInfo.camera.proj;");
		declareFunction(u8"vec4f", u8"GetVertexPosition", {}, u8"return vec4(in_POSITION, 1.0);");
		
		break;
	}
	case Shader::Class::FRAGMENT: {
		declareFunction(u8"mat4", u8"GetInstancePosition", {}, u8"return invocationInfo.instance.ModelMatrix;");
		declareFunction(u8"mat4", u8"GetCameraViewMatrix", {}, u8"return invocationInfo.camera.view;");
		declareFunction(u8"mat4", u8"GetCameraProjectionMatrix", {}, u8"return invocationInfo.camera.proj;");
		//declFunc(u8"vec4", u8"GetVertexPosition", {}, u8"return vec4(in_POSITION, 1.0);");

		break;
	}
	case Shader::Class::COMPUTE: break;
	default: ;
	}

	switch (shader.TargetSemantics) {
	case GAL::ShaderType::VERTEX: {
		shader.ShaderStage |= GAL::ShaderStages::VERTEX;
		declarationBlock += u8"layout(location=0) out vertexData { vec2 texture_coordinates; } vertexOut;\n";
		break;
	}
	case GAL::ShaderType::MESH:
		shader.ShaderStage |= GAL::ShaderStages::MESH;
		break;
	case GAL::ShaderType::CLOSEST_HIT:
		shader.ShaderStage |= GAL::ShaderStages::CLOSEST_HIT;
		break;
	case GAL::ShaderType::ANY_HIT:
		shader.ShaderStage |= GAL::ShaderStages::ANY_HIT;
		break;
	case GAL::ShaderType::INTERSECTION: {
		shader.ShaderStage |= GAL::ShaderStages::INTERSECTION;
		GTSL::StaticVector<StructElement, 2> parameters{};
		declareFunction(u8"vec4f", u8"GetVertexPosition", parameters, u8"return vec4(in_Position, 1.0);");

		{
			GTSL::StaticVector<StructElement, 32> elements;
			elements.EmplaceBack(u8"ptr_t", u8"MaterialData"); elements.EmplaceBack(u8"ptr_t", u8"InstanceData");
			declStruct(u8"shaderEntry", elements, false);
		}

		declarationBlock += u8"layout(shaderRecordEXT, scalar) buffer shader { shaderEntry shaderEntries[]; };\n";

		{
			GTSL::StaticVector<StructElement, 32> elements;
			elements.EmplaceBack(u8"uint16", u8"i");
			declStruct(u8"index", elements, true);
		}

		declarationBlock += u8"hitAttributeEXT vec2 hitBarycenter;\n";
		declarationBlock += u8"layout(location=0) rayPayloadInEXT vec4 payload;\n";

		break;
	}
	case GAL::ShaderType::TESSELLATION_CONTROL: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION: break;
	case GAL::ShaderType::GEOMETRY: break;
	case GAL::ShaderType::FRAGMENT: {
		shader.ShaderStage |= GAL::ShaderStages::FRAGMENT;
		declarationBlock += u8"layout(location=0) in vertexData { vec2 texture_coordinates; } vertexIn;\n";

		declareFunction(u8"vec2f", u8"GetFragmentPosition", {}, u8"return gl_FragCoord.xy;");
		declareFunction(u8"vec2f", u8"GetVertexTextureCoordinates", {}, u8"return vertexIn.texture_coordinates;");

		break;
	}
	case GAL::ShaderType::COMPUTE: {
		shader.ShaderStage |= GAL::ShaderStages::COMPUTE;
		declareFunction(u8"uvec2", u8"GetScreenPosition", {}, u8"return gl_WorkGroupID.xy;");
		break;
	}
	case GAL::ShaderType::TASK:
		shader.ShaderStage |= GAL::ShaderStages::TASK;
		break;
	case GAL::ShaderType::RAY_GEN: {
		shader.ShaderStage |= GAL::ShaderStages::RAY_GEN;
		declarationBlock += u8"layout(location=0) rayPayloadEXT vec4 payload;\n";

		{
			GTSL::StaticVector<StructElement, 32> elements;
			//for (uint8 i = 0; i < shader.Outputs.GetLength(); ++i) { elements.EmplaceBack(shader.Outputs[i]); }
			declStruct(u8"_renderPass", elements, true);
		}
		
		{
			GTSL::StaticVector<StructElement, 32> elements;
			elements.EmplaceBack(u8"uint64", u8"AccelerationStructure");
			elements.EmplaceBack(u8"uint32", u8"RayFlags");
			elements.EmplaceBack(u8"uint32", u8"SBTRecordOffset"); elements.EmplaceBack(u8"uint32", u8"SBTRecordStride"); elements.EmplaceBack(u8"uint32", u8"MissIndex"); elements.EmplaceBack(u8"uint32", u8"Payload");
			elements.EmplaceBack(u8"float32", u8"tMin");
			elements.EmplaceBack(u8"float32", u8"tMax");
			declStruct(u8"_rayTrace", elements, true);
		}

		declareFunction(u8"vec2f", u8"GetFragmentPosition", {}, u8"const vec2 pixelCenter = vec2(gl_LaunchIDEXT.xy) + vec2(0.5f);\nreturn pixelCenter / vec2(gl_LaunchSizeEXT.xy);");

		{
			GTSL::StaticVector<StructElement, 2> parameters{ { u8"vec3", u8"origin" }, { u8"vec3", u8"direction" } };
			declareFunction(u8"void", u8"TraceRay", parameters, u8"_rayTrace r = _rayTrace(invocationInfo.RayDispatchData);\ntraceRayEXT(accelerationStructureEXT(r.AccelerationStructure), r.RayFlags, 0xff, r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, origin, r.tMin, direction, r.tMax, r.Payload);");
		}

		break;
	}
	case GAL::ShaderType::MISS: {
		shader.ShaderStage |= GAL::ShaderStages::MISS;
		declarationBlock += u8"layout(location=0) rayPayloadInEXT vec4 payload;\n";
		break;
	}
	case GAL::ShaderType::CALLABLE:
		shader.ShaderStage |= GAL::ShaderStages::CALLABLE;
		break;
	}
	
	declareFunction(u8"vec3f", u8"Barycenter", { { u8"vec2f", u8"coords" } }, u8"return vec3(1.0f - coords.x - coords.y, coords.x, coords.y);");
	declareFunction(u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"vec2f", u8"texCoord" } }, u8"return texture(textures[nonuniformEXT(tex.Instance)], texCoord);");
	declareFunction(u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"uvec2", u8"pos" } }, u8"return texelFetch(textures[nonuniformEXT(tex.Instance)], ivec2(pos), 0);");
	declareFunction(u8"vec4f", u8"Sample", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" } }, u8"return imageLoad(images[nonuniformEXT(img.Instance)], ivec2(pos));");
	declareFunction(u8"void", u8"Write", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" }, { u8"vec4f", u8"value" } }, u8"imageStore(images[nonuniformEXT(img.Instance)], ivec2(pos), value);");
	declareFunction(u8"float32", u8"X", { { u8"vec4f", u8"vec" } }, u8"return vec.x;");
	declareFunction(u8"float32", u8"Y", { { u8"vec4f", u8"vec" } }, u8"return vec.y;");
	declareFunction(u8"float32", u8"Z", { { u8"vec4f", u8"vec" } }, u8"return vec.z;");
	declareFunction(u8"vec3f", u8"FresnelSchlick", { { u8"float32", u8"cosTheta" }, { u8"vec3f", u8"F0" } }, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
	declareFunction(u8"vec3f", u8"Normalize", { { u8"vec3f", u8"a" } }, u8"return normalize(a);");
	declareFunction(u8"float32", u8"Sigmoid", { { u8"float32", u8"x" } }, u8"return 1.0 / (1.0 + pow(x / (1.0 - x), -3.0));");
	//{
	//	GTSL::StaticVector<GTSL::Pair<GTSL::StaticString<32>, GTSL::StaticString<32>>, 3> parameters{ { u8"vec2", u8"texture_coordinate" }, { u8"float", u8"depth_from_depth_buffer" }, { u8"mat4", u8"inverse_projection_matrix" } };
	//	declFunc(u8"vec3", u8"WorldPosFromDepth", parameters, u8"vec3 clip_space_position = vec3(texture_coordinate, depth_from_depth_buffer) * 2.0 - vec3(1.0);\nvec4 view_position = vec4(vec2(inverse_projection_matrix[0][0], inverse_projection_matrix[1][1]) * clip_space_position.xy, inverse_projection_matrix[2][3] * clip_space_position.z + inverse_projection_matrix[3][3]);\nreturn (view_position.xyz / view_position.w);\n");
	//}	

	{ //main
		switch (shader.TargetSemantics) {
		case GAL::ShaderType::COMPUTE: {
			GTSL::Extent3D size = shader.threadSize;
			mainBlock += u8"layout(local_size_x="; ToString(mainBlock, size.Width);
			mainBlock += u8", local_size_y="; ToString(mainBlock, size.Height);
			mainBlock += u8", local_size_z="; ToString(mainBlock, size.Depth);
			mainBlock += u8") in;\n";
			break;
		}
		case GAL::ShaderType::MESH: {
			mainBlock += u8"layout(local_size_x="; ToString(mainBlock, 32); mainBlock += u8") in;\n";
			mainBlock += u8"layout(triangles) out;\n";
			mainBlock += u8"layout(max_vertices=64, max_primitives=126) out;\n";
			break;
		}
		}

		mainBlock += u8"void main() {\n";

		switch (shader.TargetSemantics) {
		case GAL::ShaderType::VERTEX: {
			mainBlock += u8"vertexOut.texture_coordinates = in_TEXTURE_COORDINATES;\n";
			break;
		}
		case GAL::ShaderType::FRAGMENT: break;
		case GAL::ShaderType::COMPUTE: break;
		case GAL::ShaderType::TASK: break;
		case GAL::ShaderType::MESH: break;
		case GAL::ShaderType::RAY_GEN: {
			mainBlock += u8"payload = vec4(1.0f, 0.0f, 0.0f, 1.0f);\n";
			break;
		}
		case GAL::ShaderType::ANY_HIT: break;
		case GAL::ShaderType::CLOSEST_HIT: {
			mainBlock += u8"payload = vec4(1.0f, 0.0f, 0.0f, 1.0f);\n";
			mainBlock += u8"StaticMeshPointer instance = StaticMeshPointer(shaderEntries[gl_InstanceCustomIndexEXT]);\n";
			mainBlock += u8"uint indeces[3] = uint[3](instance.IndexBuffer[3 * gl_PrimitiveID + 0], instance.IndexBuffer[3 * gl_PrimitiveID + 1], instance.IndexBuffer[3 * gl_PrimitiveID + 2]);\n";
			mainBlock += u8"vertex vertices[3] = vertex[](instance.VertexBuffer[indeces[0]], instance.VertexBuffer[indeces[1]], instance.VertexBuffer[indeces[2]]);\n";
			mainBlock += u8"const vec3 barycenter = Barycenter(hitBarycenter);\n";
			mainBlock += u8"vec3 normal = vertices[0].Normal * barycenter.x + vertices[1].Normal * barycenter.y + vertices[2].Normal * barycenter.z;\n";
			mainBlock += u8"vec2 texCoord = vertices[0].TexCoords * barycenter.x + vertices[1].TexCoords * barycenter.y + vertices[2].TexCoords * barycenter.z;\n";
			break;
		}
		case GAL::ShaderType::MISS: {
			mainBlock += u8"payload = vec4(0.0f, 0.0f, 0.0f, 1.0f);\n";
			break;
		}
		case GAL::ShaderType::INTERSECTION: break;
		case GAL::ShaderType::CALLABLE: break;
		default: ;
		}

		for(auto& e : pipeline.parameters) {
			mainBlock += resolveTypeName(e.Type); mainBlock += u8' ';
			mainBlock += e.Name; mainBlock += u8" = ";
			mainBlock += u8"invocationInfo.shader_parameters."; mainBlock += e.Name; mainBlock += u8";\n";
		}

		for (uint32 i = 0; i < shader.statements.GetLength(); ++i) {
			placeNode(shader.statements[i].begin(), 0, placeNode);
			mainBlock += u8";\n";
		}

		mainBlock += u8"}";
	}

	GTSL::StaticString<8192> fin;

	fin += headerBlock;
	fin += structBlock;
	fin += declarationBlock;
	fin += functionBlock;
	fin += mainBlock;

	return fin;
}

inline GTSL::Pair<GTSL::StaticString<8192>, Shader> GenerateShader(const GTSL::StringView jsonShader, const GPipeline& pipeline) {
	GTSL::Buffer json_deserializer(BE::TAR(u8"GenerateShader"));
	auto json = Parse(jsonShader, json_deserializer);

	Shader::Class type;

	switch (Hash(json[u8"type"])) {
	case GTSL::Hash(u8"Vertex"): type = Shader::Class::VERTEX; break;
	case GTSL::Hash(u8"Fragment"): type = Shader::Class::FRAGMENT; break;
	case GTSL::Hash(u8"Compute"): type = Shader::Class::COMPUTE; break;
	}

	Shader shader(json[u8"name"], type);

	switch (Hash(json[u8"outputSemantics"])) {
	case GTSL::Hash(u8"Compute"): {
		GTSL::StaticVector<uint16, 3> localSize;

		if (auto res = json[u8"localSize"]) {
			shader.SetThreadSize({ static_cast<uint16>(res[0].GetUint()), static_cast<uint16>(res[1].GetUint()), static_cast<uint16>(res[2].GetUint()) });
		} else {
			shader.SetThreadSize({ 1, 1, 1 });
		}

		shader.TargetSemantics = GAL::ShaderType::COMPUTE;
		break;
	}
	case GTSL::Hash(u8"Vertex"): {
		shader.TargetSemantics = GAL::ShaderType::VERTEX;
		break;
	}
	case GTSL::Hash(u8"Fragment"): {
		shader.TargetSemantics = GAL::ShaderType::FRAGMENT;
		break;
	}
	}

	for (auto e : json[u8"inputs"]) {
		shader.Layers.EmplaceBack(StructElement{ GTSL::StringView(e[u8"type"]), GTSL::StringView(e[u8"name"]) });
	}

	if(auto vertexElements = json[u8"vertexElements"]) {
		for(auto ve : vertexElements) {
			auto& e = shader.VertexElements.EmplaceBack();
			e.Identifier = ve[u8"id"];

			switch (Hash(ve[u8"type"])) {
			case GTSL::Hash(u8"float3"):
				e.Type = GAL::ShaderDataType::FLOAT3;
				break;
			case GTSL::Hash(u8"float2"):
				e.Type = GAL::ShaderDataType::FLOAT2;
				break;
			}
		}
	}

	if (auto sv = json[u8"shaderVariables"]) {
		for (auto e : sv) {
			StructElement struct_element;

			struct_element.Type = e[u8"type"];
			struct_element.Name = e[u8"name"];

			if (auto res = e[u8"defaultValue"]) {
				struct_element.DefaultValue = res;
			}

			shader.ShaderParameters.EmplaceBack(struct_element);
		}
	}

	auto parseStatement = [](GTSL::JSONMember parent, GTSL::Tree<ShaderNode, BE::PAR>& tree, auto& levels, auto&& self) -> void {
		uint32 parentHandle = 0;

		if (levels) {
			parentHandle = levels.back();
		}

		auto handle = tree.Emplace(parentHandle);
		auto& node = tree[handle];

		levels.EmplaceBack(handle);

		if (auto nameMember = parent[u8"name"]) { //var || var decl || func || operator
			node.Name = GTSL::StringView(nameMember);

			if (auto paramsMember = parent[u8"params"]) { //function, var decl
				if (auto typeMember = parent[u8"type"]) { //name ^ params ^ type -> var decl
					node.ValueType = ShaderNode::Type::VAR_DEC;
					node.TypeName = GTSL::StringView(typeMember);
				} else { //name ^ params ^ ~type -> function
					if (GTSL::IsSymbol(nameMember.GetStringView()[0])) {
						node.ValueType = ShaderNode::Type::OPERATOR;
					} else if (nameMember.GetStringView() == u8"vertexPosition" or nameMember.GetStringView() == u8"fragmentColor") {
						node.ValueType = ShaderNode::Type::SHADER_RESULT;
					} else {
						node.ValueType = ShaderNode::Type::FUNCTION;
					}
				}

				for (auto e : parent[u8"params"]) {
					self(e, tree, levels, self);
				}
			} else { //name and no params -> var
				node.ValueType = ShaderNode::Type::VARIABLE;
			}
		} else { //no name -> literal
			node.ValueType = ShaderNode::Type::LITERAL;
		}

		levels.PopBack();
	};

	if (auto fs = json[u8"functions"]) {
		for (auto f : fs) {
			f[u8"return"];
			f[u8"name"];

			for (auto p : f[u8"params"]) {
				p[u8"type"];
				p[u8"name"];
			}

			GTSL::StaticVector<uint32, 8> levels;

			for(auto s : f[u8"statements"]) {
				//shader.functions.EmplaceBack(BE::PAR(u8"ShaderGenerator")); //allocator
				//parseStatement(s, shader.functions.back(), levels, parseStatement);
			}
		}
	}

	{
		GTSL::StaticVector<uint32, 8> levels;

		for (auto e : json[u8"statements"]) {
			shader.statements.EmplaceBack(BE::PAR(u8"ShaderGenerator")); //allocator
			parseStatement(e, shader.statements.back(), levels, parseStatement);
		}
	}

	return { GenerateShader(shader, pipeline), GTSL::MoveRef(shader) };
}

#include <spirv-headers/spirv.hpp>

inline void GenSPIRV() {
	GTSL::StaticVector<uint32, 1024> spirv;

	const bool debugMode = true; uint32 id = 0;

	//first words
	spirv.EmplaceBack(spv::MagicNumber); //SPIR-V magic num
	spirv.EmplaceBack(0 << 24 | 1 << 16 | 5 << 8 | 0); //SPIR-V version number
	spirv.EmplaceBack(0); //SPIR-V generator number
	spirv.EmplaceBack(0); //bound
	spirv.EmplaceBack(0); //instruction schema

	auto addInst = [&]<typename... ARGS>(uint16 enumerant, ARGS&&... args) {
		auto wordCount = uint16(1) + static_cast<uint16>(sizeof...(ARGS));
		spirv.EmplaceBack(wordCount << 16 | enumerant);
		(spirv.EmplaceBack(args), ...);
	};

	auto addInstVar = [&]<typename... ARGS>(uint16 enumerant, GTSL::Range<const uint32*> words) {
		auto wordCount = uint16(1) + static_cast<uint16>(words.ElementCount());
		spirv.EmplaceBack(wordCount << 16 | enumerant);
		spirv.PushBack(words);
	};

	auto packString = [&](const GTSL::StringView string, auto& cont) {
		for (uint32 u = 0; u < string.GetBytes(); ++u) {
			uint32& ch = cont.EmplaceBack(0u);

			for (uint32 t = 0; t < 4 && u < string.GetBytes(); ++t, ++u) {
				ch |= static_cast<uint32>(string[u + t]) << (t * 8);
			}
		}

		if(GTSL::ModuloByPowerOf2(string.GetBytes(), 4) == 0) { //if all non null terminator characters are a multiple of 4 bytes that means that all groups of four bytes where put in an int and no free byte was left to represent a null terminator
			cont.EmplaceBack(0u);
		}
	};

	//capability section
	addInst(spv::OpCapability, spv::Capability::CapabilityInt64);
	addInst(spv::OpCapability, spv::Capability::CapabilityInt16);
	addInst(spv::OpCapability, spv::Capability::CapabilityImageReadWrite);
	addInst(spv::OpCapability, spv::Capability::CapabilitySampledImageArrayDynamicIndexing);
	addInst(spv::OpCapability, spv::Capability::CapabilitySampledImageArrayNonUniformIndexing);
	addInst(spv::OpCapability, spv::Capability::CapabilityStorageImageArrayDynamicIndexing);
	addInst(spv::OpCapability, spv::Capability::CapabilityStorageImageArrayNonUniformIndexing);
	addInst(spv::OpCapability, spv::Capability::CapabilityVariablePointers);
	addInst(spv::OpCapability, spv::Capability::CapabilityVariablePointersStorageBuffer);
	addInst(spv::OpCapability, spv::Capability::CapabilityPhysicalStorageBufferAddresses);
	
	//extension section
	//memory model section
	addInst(spv::OpMemoryModel, spv::AddressingModel::AddressingModelPhysical64, spv::MemoryModel::MemoryModelVulkan);
	//entry points section

	//Interface is a list of <id> of global OpVariable instructions.
	//These declare the set of global variables from a module that form the interface of this entry point.
	//The set of Interface <id> must be equal to or a superset of the global OpVariable Result <id> referenced by the entry point’s static call tree,
	//within the interface’s storage classes. Before version 1.4, the interface’s storage classes are limited to the Input and Output storage classes.
	//Starting with version 1.4, the interface’s storage classes are all storage classes used in declaring all global variables
	//referenced by the entry point’s call tree.
	//Interface <id> are forward references.Before version 1.4, duplication of these <id> is tolerated.Starting with version 1.4,
	//an <id> must not appear more than once.
	addInst(spv::OpEntryPoint, spv::ExecutionModelVertex, 0/*Result<id> of an OpFunction*/, 0/*literal name*/);
	//execution modes section

	auto declStruct = [&](const GTSL::StringView name, const GTSL::Range<const StructElement*> params) {
		auto structId = id++;

		if(debugMode) {
			GTSL::StaticVector<uint32, 32> chs;
			chs.EmplaceBack(structId);
			packString(name, chs);
			addInstVar(spv::OpName, chs);
		}

		uint32 byteOffset = 0;
		for (uint32 i = 0; auto & e : params) {
			addInst(spv::OpMemberDecorate, structId, i, byteOffset);

			if (debugMode) { //decorate struct members with name if compiling in debug
				GTSL::StaticVector<uint32, 32> chs;

				chs.EmplaceBack(structId);
				chs.EmplaceBack(i);

				packString(e.Name, chs);

				addInstVar(spv::OpMemberName, chs);
			}

			byteOffset += 1;
			i += 1;
		}
	};

	auto declFunc = [&](const GTSL::StringView ret, const GTSL::Range<const StructElement*> params) {
		auto functionId = id++;

		addInst(spv::OpFunction);

		for(auto& e : params) {
			addInst(spv::OpFunctionParameter);
		}

		addInst(spv::OpReturn);
		addInst(spv::OpFunctionEnd);
	};
}