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

//Object types are always stored as the interface types, not the end target's name
struct StructElement {
	StructElement(const GTSL::StringView t, const GTSL::StringView n) : Type(t), Name(n) {}
	StructElement(const GTSL::StringView t, const GTSL::StringView n, const GTSL::StringView dv) : Type(t), Name(n), DefaultValue(dv) {}

	GTSL::ShortString<64> Type, Name, DefaultValue;
};

struct ShaderNode {
	enum class Type : uint8 {
		VARIABLE, VAR_DEC, FUNCTION, SHADER_RESULT, OPERATOR, LITERAL, SHADER_PARAMETER, RETURN, RVALUE
	} ValueType;

	GTSL::ShortString<64> Name, TypeName;

	auto GetName() const -> GTSL::StringView {
		return Name;
	}
};

bool IsAnyOf(const auto& a, const auto&... elems) {
	return ((a == elems) or ...);
}

inline auto parseStatement(GTSL::JSONMember parent, GTSL::Tree<ShaderNode, BE::PAR>& tree, uint32 parentHandle) -> uint32 {
	auto handle = tree.Emplace(parentHandle);
	auto& node = tree[handle];

	if (auto nameMember = parent[u8"name"]) { //var || var decl || func || operator
		node.Name = GTSL::StringView(nameMember);

		if (auto paramsMember = parent[u8"params"]) { //function, var decl
			if (auto typeMember = parent[u8"type"]) { //name ^ params ^ type -> var decl
				node.ValueType = ShaderNode::Type::VAR_DEC;
				node.TypeName = GTSL::StringView(typeMember);
			}
			else { //name ^ params ^ ~type -> function
				if (GTSL::IsSymbol(nameMember.GetStringView()[0])) {
					node.ValueType = ShaderNode::Type::OPERATOR;
				}
				else if (nameMember.GetStringView() == u8"return") {
					node.ValueType = ShaderNode::Type::RETURN;
				}
				else {
					node.ValueType = ShaderNode::Type::FUNCTION;
				}
			}

			for (auto e : parent[u8"params"]) {
				parseStatement(e, tree, handle);
			}
		}
		else { //name and no params -> var
			node.ValueType = ShaderNode::Type::VARIABLE;
		}
	}
	else if (auto outputMember = parent[u8"output"]) {
		node.Name = outputMember;
		node.ValueType = ShaderNode::Type::SHADER_RESULT;
		for (auto e : parent[u8"params"]) {
			parseStatement(e, tree, handle);
		}
	}
	else { //no name -> literal
		if (auto valueMember = parent[u8"value"]) {
			node.Name = valueMember;
			node.ValueType = ShaderNode::Type::LITERAL;
		}
		else {
			node.TypeName = parent[u8"type"];
			node.ValueType = ShaderNode::Type::RVALUE;
			for (auto e : parent[u8"params"]) {
				parseStatement(e, tree, handle);
			}
		}
	}

	return handle;
}

struct Shader {
	enum class Class { VERTEX, SURFACE, COMPUTE, RENDER_PASS, RAY_GEN, MISS };

	Shader(const GTSL::StringView name, const Class clss) : Name(name), Type(clss) {}

	void AddShaderParameter(const StructElement element) { ShaderParameters.EmplaceBack(element); }

	void SetThreadSize(const GTSL::Extent3D size) { threadSize = size; }

	GTSL::ShortString<32> Name;
	Class Type;

	GTSL::StaticVector<StructElement, 8> ShaderParameters;

	//compute
	GTSL::Extent3D threadSize;

	GTSL::StaticVector<GTSL::Tree<ShaderNode, BE::PAR>, 8> statements;
	GAL::ShaderType TargetSemantics;
	bool Transparency = false
	;

	struct FunctionDefinition {
		GTSL::StaticString<32> Return, Name;
		GTSL::StaticVector<StructElement, 8> Parameters;
		GTSL::StaticVector<GTSL::Tree<ShaderNode, BE::PAR>, 8> Statements;
	};
	GTSL::StaticVector<FunctionDefinition, 8> Functions;
};

struct GPipeline
{
	GTSL::StaticVector<GAL::Pipeline::VertexElement, 32> VertexElements;
	GTSL::StaticVector<StructElement, 16> Interface;

	GTSL::StaticVector<GTSL::StaticVector<GTSL::StaticString<64>, 8>, 8> descriptors;
	GTSL::StaticVector<StructElement, 8> parameters;

	GTSL::StaticVector<StructElement, 8> ShaderRecord[4];

	GTSL::StaticVector<StructElement, 8> Layers;
	GTSL::StaticVector<StructElement, 8> Outputs;

	struct StructDefinition {
		GTSL::StaticString<32> Name;
		GTSL::StaticVector<StructElement, 8> Members;
	};
	GTSL::StaticVector<StructDefinition, 8> Structs;

	struct FunctionDefinition {
		GTSL::StaticString<32> Return, Name;
		GTSL::StaticVector<StructElement, 8> Parameters;
		GTSL::StaticVector<GTSL::Tree<ShaderNode, BE::PAR>, 8> Statements;
	};
	GTSL::StaticVector<FunctionDefinition, 8> Functions;

	GTSL::ShortString<32> TargetSemantics;

	GAL::IndexType IndexType = GAL::IndexType::UINT16;
};

inline GTSL::StaticString<8192> GenerateShader(Shader& shader, const GPipeline& pipeline) {
	GTSL::StaticString<2048> headerBlock, structBlock, functionBlock, declarationBlock, mainBlock;

	headerBlock += u8"#version 460 core\n"; //push version

	switch (Hash(pipeline.TargetSemantics)) {
	case GTSL::Hash(u8"raster"): {
		switch (shader.Type) {
		case Shader::Class::VERTEX: shader.TargetSemantics = GAL::ShaderType::VERTEX; break;
		case Shader::Class::SURFACE: shader.TargetSemantics = GAL::ShaderType::FRAGMENT; break;
		case Shader::Class::COMPUTE: break;
		}
		break;
	}
	case GTSL::Hash(u8"compute"): {
		switch (shader.Type) {
		case Shader::Class::VERTEX: shader.TargetSemantics = GAL::ShaderType::COMPUTE; break;
		case Shader::Class::SURFACE: shader.TargetSemantics = GAL::ShaderType::COMPUTE; break;
		case Shader::Class::COMPUTE: shader.TargetSemantics = GAL::ShaderType::COMPUTE; break;
		}
		break;
	}
	case GTSL::Hash(u8"rayTrace"):
		headerBlock += u8"#extension GL_EXT_ray_tracing : enable\n";
		switch (shader.Type) {
		case Shader::Class::VERTEX: shader.TargetSemantics = GAL::ShaderType::COMPUTE; break;
		case Shader::Class::SURFACE: shader.TargetSemantics = shader.Transparency ? GAL::ShaderType::ANY_HIT : GAL::ShaderType::CLOSEST_HIT; break;
		case Shader::Class::COMPUTE: shader.TargetSemantics = GAL::ShaderType::RAY_GEN; break;
		}
		break;
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

	for (uint32 si = 0; auto & s : pipeline.descriptors) {
		for (uint32 bi = 0; auto & b : s) {
			declarationBlock += u8"layout(set="; ToString(declarationBlock, si);
			declarationBlock += u8",binding="; ToString(declarationBlock, bi); declarationBlock += u8") ";
			declarationBlock += b; declarationBlock += u8";\n";
			++bi;
		}
		++si;
	}

	struct ShaderFunction {
		struct FunctionSignature {
			GTSL::StaticVector<StructElement, 8> Parameters;
			GTSL::ShortString<32> ReturnType;
			GTSL::StaticString<256> Body;
			bool Used = false;
			bool IsDrawCallConstant = true;
		};

		GTSL::StaticVector<FunctionSignature, 8> FunctionVersions;
	};

	GTSL::HashMap<Id, ShaderFunction, GTSL::DefaultAllocatorReference> functions(16, 1.0f);
	GTSL::HashMap<Id, GTSL::StaticVector<StructElement, 32>, GTSL::DefaultAllocatorReference> structs(16, 1.0f);

	struct VariableDeclaration {
		StructElement Element;
	};
	GTSL::HashMap<Id, VariableDeclaration, GTSL::DefaultAllocatorReference> variables(16, 1.0f);

	auto resolveTypeName = [&](const GTSL::StringView name) -> GTSL::StaticString<32> {
		switch (Hash(name)) {
		case GTSL::Hash(u8"float32"): return GTSL::StringView(u8"float");
		case GTSL::Hash(u8"vec2f"): return GTSL::StringView(u8"vec2");
		case GTSL::Hash(u8"vec3f"): return GTSL::StringView(u8"vec3");
		case GTSL::Hash(u8"vec4f"): return GTSL::StringView(u8"vec4");
		case GTSL::Hash(u8"mat4f"): return GTSL::StringView(u8"mat4");
		case GTSL::Hash(u8"uint64"): return GTSL::StringView(u8"uint64_t");
		case GTSL::Hash(u8"uint32"): return GTSL::StringView(u8"uint");
		case GTSL::Hash(u8"uint16"): return GTSL::StringView(u8"uint16_t");
		case GTSL::Hash(u8"ptr_t"): return GTSL::StringView(u8"uint64_t");
		}

		if (*(name.end() - 1) == u8'*') {
			GTSL::StaticString<32> n(name);
			DropLast(n, u8'*');
			n += u8"Pointer";
			return n;
		}

		return name;
	};

	auto addStructElement = [resolveTypeName](GTSL::StaticString<2048>& string, const StructElement& element) {
		string += resolveTypeName(element.Type); string += u8' '; string += element.Name; string += u8';';
	};

	auto addStructElement2 = [resolveTypeName](GTSL::StaticString<2048>& string, const StructElement& element, const GTSL::StringView prefix) {
		string += resolveTypeName(element.Type); string += u8' '; string += prefix; string += element.Name; string += u8';';
	};

	auto addVariable = [&](const GTSL::StringView interfaceName, const StructElement& element) {
		variables.Emplace(interfaceName, element);
	};

	auto declareFunction = [&](const GTSL::StringView ret, const GTSL::StringView name, GTSL::Range<const StructElement*> parameters, const GTSL::StringView impl) {
		auto functionByName = functions.TryEmplace(Id(name));

		if (!functionByName) {
			auto eq = true;

			for (uint32 f = 0; f < functionByName.Get().FunctionVersions.GetLength(); ++f) {
				bool et = true;

				if (parameters.ElementCount() == GTSL::Range<const StructElement*>(functionByName.Get().FunctionVersions[f].Parameters).ElementCount()) {
					for (uint64 i = 0; i < parameters.ElementCount(); ++i) {
						if (parameters[i].Type != functionByName.Get().FunctionVersions[f].Parameters[i].Type or parameters[i].Name != functionByName.Get().FunctionVersions[f].Parameters[i].Name) { et = false; break; }
					}
				}
				else {
					et = false;
				}

				BE_ASSERT(!et, u8"Already exists");
			}
		}

		auto& functionVersion = functionByName.Get().FunctionVersions.EmplaceBack();
		functionVersion.ReturnType = ret;

		for (auto& e : parameters) { functionVersion.Parameters.EmplaceBack(e); }

		functionVersion.Body = impl;
	};

	auto declareStruct = [&](GTSL::StringView ne, GTSL::Range<const StructElement*> structElements, bool ref, bool readOnly = true) {
		GTSL::StaticString<32> name(ne);

		if (ref) { name += u8"Pointer"; }

		auto& st = structs.Emplace(Id(name));

		for (auto& e : structElements) {
			st.EmplaceBack(e);
		}

		if (!structElements.ElementCount()) {
			st.EmplaceBack(u8"uint32", u8"dummy");
		}

		if (ref) {
			structBlock += u8"layout(buffer_reference,scalar,buffer_reference_align=4) ";

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

	using TTT = decltype(static_cast<const GTSL::Tree<ShaderNode, BE::PAR>&>(shader.statements[0]).begin());

	auto placeNode = [resolveTypeName, useFunction, &shader](GTSL::StaticString<2048>& string, TTT nodeHandle, uint32_t level, auto&& self) -> void {
		ShaderNode* node = &nodeHandle.Get();

		auto genFuncCall = [&](GTSL::StringView name, GTSL::StringView opener, GTSL::StringView divider, GTSL::StringView closer) {
			string += name; string += opener;

			for (uint32 i = 0; auto e : nodeHandle) {
				self(string, e, level + 1, self);
				if (i < nodeHandle.GetLength() - 1) { string += divider; }
				++i;
			}

			string += closer;
		};

		switch (node->ValueType) {
		case ShaderNode::Type::VARIABLE: {
			string += node->Name;
			break;
		}
		case ShaderNode::Type::VAR_DEC: {
			string += resolveTypeName(node->TypeName); string += u8' '; string += node->Name;
			break;
		}
		case ShaderNode::Type::FUNCTION: {
			useFunction(node->Name);
			genFuncCall(node->Name, u8"(", u8", ", u8")");
			break;
		}
		case ShaderNode::Type::OPERATOR: {
			genFuncCall(u8"", u8"", node->Name, u8"");
			break;
		}
		case ShaderNode::Type::LITERAL: {
			string += node->Name;
			break;
		}
		case ShaderNode::Type::RVALUE: {
			genFuncCall(resolveTypeName(node->TypeName), u8"(", u8", ", u8")");
			break;
		}
		case ShaderNode::Type::SHADER_RESULT: {
			GTSL::StaticMap<Id, GTSL::StaticString<64>, 8> results(8);

			if (shader.TargetSemantics == GAL::ShaderType::VERTEX) {
				results.Emplace(u8"vertexPosition", u8"gl_Position");
			}

			results.Emplace(u8"surfaceColor", u8"out_Color");
			results.Emplace(u8"surfaceNormal", u8"out_Normal");
			results.Emplace(u8"surfacePosition", u8"out_Position");

			//navigate by interface name
			//variableByInterface(u8"surfaceColor") = out_Color;

			string += results[Id(node->Name)]; string += u8" = ";

			for (uint32 i = 0; auto e : nodeHandle) {
				self(string, e, level + 1, self);
				++i;
			}

			break;
		}
		case ShaderNode::Type::SHADER_PARAMETER: {
			string += u8"invocationInfo.shader_parameters."; string += node->Name;
			break;
		}
		case ShaderNode::Type::RETURN: {
			string += u8"return ";

			for (uint32 i = 0; auto e : nodeHandle) {
				self(string, e, level + 1, self);
				++i;
			}

			break;
		}
		}
	};

	auto shaderDataTypeToType = [](const GAL::ShaderDataType type) -> GTSL::ShortString<32> {
		switch (type) {
		case GAL::ShaderDataType::FLOAT:  return u8"float32";
		case GAL::ShaderDataType::FLOAT2: return u8"vec2f";
		case GAL::ShaderDataType::FLOAT3: return u8"vec3f";
		case GAL::ShaderDataType::FLOAT4: return u8"vec4f";
		case GAL::ShaderDataType::UINT16: return u8"uint16_t";
		case GAL::ShaderDataType::UINT32: return u8"uint32";
		case GAL::ShaderDataType::INT:    return u8"int32";
		case GAL::ShaderDataType::INT2: break;
		case GAL::ShaderDataType::INT3: break;
		case GAL::ShaderDataType::INT4: break;
		case GAL::ShaderDataType::BOOL: break;
		case GAL::ShaderDataType::MAT3: break;
		case GAL::ShaderDataType::MAT4: break;
		default:;
		}
	};

	GTSL::StaticVector<StructElement, 32> vertexElements;

	for (const auto& e : pipeline.VertexElements) {
		vertexElements.EmplaceBack(shaderDataTypeToType(e.Type), e.Identifier);
	}

	declareStruct(u8"vertex", vertexElements, false);
	declareStruct(u8"vertex", vertexElements, true);

	declareStruct(u8"index", { { u8"uint16", u8"i" } }, true);

	switch (shader.Type) {
	case Shader::Class::VERTEX: {
		declareFunction(u8"vec4f", u8"GetVertexPosition", {}, u8"return vec4(in_POSITION, 1.0);");

		if (shader.TargetSemantics == GAL::ShaderType::VERTEX) {
			for (uint8 i = 0; const auto & ve : pipeline.VertexElements) {
				GTSL::StaticString<64> name(u8"in_"); name += ve.Identifier;

				declarationBlock += u8"layout(location="; ToString(declarationBlock, i); declarationBlock += u8") in ";
				declarationBlock += resolveTypeName(shaderDataTypeToType(ve.Type));
				declarationBlock += u8' '; declarationBlock += name; declarationBlock += u8";\n";
				++i;

				addVariable(name, { shaderDataTypeToType(ve.Type), name });
			}
		}

		break;
	}
	case Shader::Class::SURFACE: {
		if (shader.TargetSemantics == GAL::ShaderType::FRAGMENT) {
			for (uint32 i = 0; auto & o : pipeline.Outputs) {
				GTSL::StaticString<64> name(u8"out_"); name += o.Name;

				declarationBlock += u8"layout(location="; ToString(declarationBlock, i); declarationBlock += u8") out ";
				addStructElement(declarationBlock, { o.Type, name, o.DefaultValue }); declarationBlock += u8'\n';
				++i;

				addVariable(name, o);
			}
		}

		break;
	}
	}

	for (const auto& s : pipeline.Structs) {
		declareStruct(s.Name, s.Members, false);
		declareStruct(s.Name, s.Members, true);
	}

	declareStruct(u8"shaderParametersData", pipeline.parameters, true);

	for (const auto& f : pipeline.Functions) {
		GTSL::StaticString<2048> fImpl;

		for (auto& s : f.Statements) {
			placeNode(fImpl, s.begin(), 0, placeNode); fImpl += u8";";
		}

		declareFunction(f.Return, f.Name, f.Parameters, fImpl);
	}

	{ //push constant
		declarationBlock += u8"layout(push_constant, scalar) uniform _invocationInfo { ";

		addVariable(u8"invocationInfo", { u8"push_constant", u8"invocationInfo"});

		for (const auto& l : pipeline.Layers) {
			declarationBlock += resolveTypeName(l.Type); declarationBlock += u8' '; declarationBlock += l.Name; declarationBlock += u8"; ";
		}
		declarationBlock += u8"} invocationInfo;\n";
	}

	switch (Hash(pipeline.TargetSemantics)) {
	break; case GTSL::Hash(u8"fragment"): {

	}
	break; case GTSL::Hash(u8"rayTrace"): {
		for (uint32 i = 0; auto & e : pipeline.Interface) {
			declarationBlock += u8"layout(location="; ToString(declarationBlock, i); declarationBlock += u8") ";
			declarationBlock += shader.TargetSemantics == GAL::ShaderType::RAY_GEN ? u8"rayPayloadEXT " : u8"rayPayloadInEXT ";
			declarationBlock += resolveTypeName(e.Type); declarationBlock += e.Name; declarationBlock += u8";\n";
			++i;
		}
	}
	}

	auto addShaderRecordDeclaration = [&](const uint8 index) {
		declarationBlock += u8"layout(shaderRecordEXT, scalar) buffer shader { ";

		addVariable(u8"shaderRecordEXT", { u8"shader", u8"invocationInfo"});

		for (auto& e : pipeline.ShaderRecord[index]) {
			addStructElement(declarationBlock, e);
		}

		declarationBlock += u8" };\n";
	};

	declareFunction(u8"vec3f", u8"Barycenter", { { u8"vec2f", u8"coords" } }, u8"return vec3(1.0f - coords.x - coords.y, coords.x, coords.y);");
	declareFunction(u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"vec2f", u8"texCoord" } }, u8"return texture(sampler2D(textures[nonuniformEXT(tex.Instance)], s), texCoord);");
	declareFunction(u8"vec4f", u8"Sample", { { u8"TextureReference", u8"tex" }, { u8"uvec2", u8"pos" } }, u8"return texelFetch(sampler2D(textures[nonuniformEXT(tex.Instance)], s), ivec2(pos), 0);");
	declareFunction(u8"vec4f", u8"Sample", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" } }, u8"return imageLoad(images[nonuniformEXT(img.Instance)], ivec2(pos));");
	declareFunction(u8"void", u8"Write", { { u8"ImageReference", u8"img" }, { u8"uvec2", u8"pos" }, { u8"vec4f", u8"value" } }, u8"imageStore(images[nonuniformEXT(img.Instance)], ivec2(pos), value);");
	declareFunction(u8"float32", u8"X", { { u8"vec4f", u8"vec" } }, u8"return vec.x;");
	declareFunction(u8"float32", u8"Y", { { u8"vec4f", u8"vec" } }, u8"return vec.y;");
	declareFunction(u8"float32", u8"Z", { { u8"vec4f", u8"vec" } }, u8"return vec.z;");
	declareFunction(u8"vec3f", u8"FresnelSchlick", { { u8"float32", u8"cosTheta" }, { u8"vec3f", u8"F0" } }, u8"return F0 + (1.0 - F0) * pow(max(0.0, 1.0 - cosTheta), 5.0);");
	declareFunction(u8"vec3f", u8"Normalize", { { u8"vec3f", u8"a" } }, u8"return normalize(a);");
	declareFunction(u8"float32", u8"Sigmoid", { { u8"float32", u8"x" } }, u8"return 1.0 / (1.0 + pow(x / (1.0 - x), -3.0));");
	declareFunction(u8"vec3f", u8"WorldPositionFromDepth", { { u8"vec2f", u8"texture_coordinate" }, { u8"float32", u8"depth_from_depth_buffer" }, { u8"mat4f", u8"inverse_projection_matrix" } }, u8"vec4 p = inverse_projection_matrix * vec4(vec3(texture_coordinate * 2.0 - vec2(1.0), depth_from_depth_buffer), 1.0); return p.xyz / p.w;\n");

	switch (shader.TargetSemantics) {
	case GAL::ShaderType::VERTEX: {
		declarationBlock += u8"layout(location=0) out vertexData { ";
		for (auto& e : pipeline.Interface) {
			addStructElement(declarationBlock, e);
		}
		declarationBlock += u8" } vertexOut;\n";
		break;
	}
	case GAL::ShaderType::MESH:
		mainBlock += u8"layout(local_size_x="; ToString(mainBlock, 32); mainBlock += u8") in;\n";
		mainBlock += u8"layout(triangles) out;\n";
		mainBlock += u8"layout(max_vertices=64, max_primitives=126) out;\n";
		break;
	case GAL::ShaderType::CLOSEST_HIT:
		declarationBlock += u8"hitAttributeEXT vec2 hitBarycenter;\n";
		declareFunction(u8"vec3f", u8"GetVertexBarycenter", {}, u8"return Barycenter(hitBarycenter);");

		declareFunction(u8"vec2f", u8"GetVertexTextureCoordinates", {}, u8"StaticMeshPointer instance = shaderEntries[gl_InstanceCustomIndexEXT]; uint16_t indices[3] = instance.IndexBuffer[3 * gl_PrimitiveID]; vertex vertices[3] = vertex[](instance.VertexBuffer[indeces[0]], instance.VertexBuffer[indeces[1]], instance.VertexBuffer[indeces[2]]); vec2 barycenter = GetVertexBarycenter(); return vertices[0].TexCoords * barycenter.x + vertices[1].TexCoords * barycenter.y + vertices[2].TexCoords * barycenter.z;");

		break;
	case GAL::ShaderType::ANY_HIT:
		break;
	case GAL::ShaderType::INTERSECTION: {
		declareFunction(u8"vec4f", u8"GetVertexPosition", {}, u8"return vec4(in_Position, 1.0);");

		{
			GTSL::StaticVector<StructElement, 32> elements;
			elements.EmplaceBack(u8"ptr_t", u8"MaterialData"); elements.EmplaceBack(u8"ptr_t", u8"InstanceData");
			declareStruct(u8"shaderEntry", elements, false);
		}

		addShaderRecordDeclaration(GAL::HIT_TABLE_INDEX);

		declareStruct(u8"index", { { pipeline.IndexType == GAL::IndexType::UINT16 ? u8"uint16" : u8"uint32", u8"i" } }, true);

		break;
	}
	case GAL::ShaderType::TESSELLATION_CONTROL: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION: break;
	case GAL::ShaderType::GEOMETRY: break;
	case GAL::ShaderType::FRAGMENT: {
		declarationBlock += u8"layout(location=0) in vertexData { ";

		for (auto& e : pipeline.Interface) {
			addStructElement(declarationBlock, e);
		}

		declarationBlock += u8" } vertexIn;\n";

		declareFunction(u8"vec2f", u8"GetFragmentPosition", {}, u8"return gl_FragCoord.xy;");
		declareFunction(u8"float32", u8"GetFragmentDepth", {}, u8"return gl_FragCoord.z;");
		declareFunction(u8"vec2f", u8"GetVertexTextureCoordinates", {}, u8"return vertexIn.textureCoordinates;");
		declareFunction(u8"mat4f", u8"GetInverseProjectionMatrix", {}, u8"return invocationInfo.camera.projInverse;");
		declareFunction(u8"vec3f", u8"GetVertexViewSpacePosition", {}, u8"return vertexIn.viewSpacePosition;");
		declareFunction(u8"vec3f", u8"GetVertexViewSpaceNormal", {}, u8"return vertexIn.viewSpaceNormal;");
		break;
	}
	case GAL::ShaderType::COMPUTE: {
		declareFunction(u8"uvec2", u8"GetScreenPosition", {}, u8"return gl_WorkGroupID.xy;");

		GTSL::Extent3D size = shader.threadSize;
		mainBlock += u8"layout(local_size_x="; ToString(mainBlock, size.Width);
		mainBlock += u8",local_size_y="; ToString(mainBlock, size.Height);
		mainBlock += u8",local_size_z="; ToString(mainBlock, size.Depth);
		mainBlock += u8") in;\n";

		break;
	}
	case GAL::ShaderType::TASK:
		break;
	case GAL::ShaderType::RAY_GEN: {
		declareFunction(u8"vec2f", u8"GetFragmentPosition", {}, u8"const vec2 pixelCenter = vec2(gl_LaunchIDEXT.xy) + vec2(0.5f);\nreturn pixelCenter / vec2(gl_LaunchSizeEXT.xy);");

		{
			GTSL::StaticVector<StructElement, 2> parameters{ { u8"vec3", u8"origin" }, { u8"vec3", u8"direction" } };
			declareFunction(u8"void", u8"TraceRay", parameters, u8"rayTraceDataPointer r = invocationInfo.RayDispatchData;\ntraceRayEXT(accelerationStructureEXT(r.AccelerationStructure), r.RayFlags, 0xff, r.SBTRecordOffset, r.SBTRecordStride, r.MissIndex, origin, r.tMin, direction, r.tMax, 0);");
		}

		addShaderRecordDeclaration(GAL::RAY_GEN_TABLE_INDEX);

		break;
	}
	case GAL::ShaderType::MISS: {
		addShaderRecordDeclaration(GAL::MISS_TABLE_INDEX);
		break;
	}
	case GAL::ShaderType::CALLABLE:
		addShaderRecordDeclaration(GAL::CALLABLE_TABLE_INDEX);

		break;
	}

	{ //main
		mainBlock += u8"void main() {\n";

		switch (shader.TargetSemantics) {
		break; case GAL::ShaderType::VERTEX: {
			mainBlock += u8"vertexOut.textureCoordinates = in_TEXTURE_COORDINATES;\n";
			mainBlock += u8"vertexOut.viewSpacePosition = vec3(GetCameraViewMatrix() * vec4(in_POSITION, 0));\n";
			mainBlock += u8"vertexOut.viewSpaceNormal = vec3(GetCameraViewMatrix() * vec4(in_NORMAL, 0));\n";
		}
		break; case GAL::ShaderType::FRAGMENT:
		break; case GAL::ShaderType::COMPUTE:
		break; case GAL::ShaderType::TASK:
		break; case GAL::ShaderType::MESH:
		break; case GAL::ShaderType::RAY_GEN:
		break; case GAL::ShaderType::ANY_HIT:
		break; case GAL::ShaderType::CLOSEST_HIT:
		break; case GAL::ShaderType::MISS:
		break; case GAL::ShaderType::INTERSECTION:
		break; case GAL::ShaderType::CALLABLE: break;
		}

		for (auto& e : pipeline.parameters) {
			mainBlock += resolveTypeName(e.Type); mainBlock += u8' ';
			mainBlock += e.Name; mainBlock += u8" = ";
			mainBlock += u8"invocationInfo.shader_parameters."; mainBlock += e.Name; mainBlock += u8";\n";
		}

		for (uint32 i = 0; i < shader.statements.GetLength(); ++i) {
			placeNode(mainBlock, static_cast<const Shader&>(shader).statements[i].begin(), 0, placeNode);
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

// SHADER DOC
// Class: Could be thought of as shader use, (Surface, Vertex, PostProcess, RayGen, Miss, Etc)
// TargetSemantics: target shader stage, (Vertex, Fragment, ClosestHit, AnyHit, Miss, Compute, Etc)
// GPipeline: defines environment for shader to operate in. Defines how common data is accessed so that the shader generator knows how to
// seamlessly translate Classes to TargetSemantics

inline GTSL::Pair<GTSL::StaticString<8192>, Shader> GenerateShader(const GTSL::StringView jsonShader, const GPipeline& pipeline) {
	GTSL::Buffer json_deserializer(BE::TAR(u8"GenerateShader"));
	auto json = Parse(jsonShader, json_deserializer);

	Shader::Class shaderClass;

	switch (Hash(json[u8"class"])) {
	case GTSL::Hash(u8"Vertex"): shaderClass = Shader::Class::VERTEX; break;
	case GTSL::Hash(u8"Surface"): shaderClass = Shader::Class::SURFACE; break;
	case GTSL::Hash(u8"Compute"): shaderClass = Shader::Class::COMPUTE; break;
	case GTSL::Hash(u8"RayGen"): shaderClass = Shader::Class::RAY_GEN; break;
	case GTSL::Hash(u8"Miss"): shaderClass = Shader::Class::MISS; break;
	}

	Shader shader(json[u8"name"], shaderClass);

	if (shaderClass == Shader::Class::COMPUTE) {
		if (auto res = json[u8"localSize"]) {
			shader.SetThreadSize({ static_cast<uint16>(res[0].GetUint()), static_cast<uint16>(res[1].GetUint()), static_cast<uint16>(res[2].GetUint()) });
		} else {
			shader.SetThreadSize({ 1, 1, 1 });
		}
	}

	if (auto sv = json[u8"shaderVariables"]) {
		for (auto e : sv) {
			StructElement struct_element(e[u8"type"], e[u8"name"]);

			if (auto res = e[u8"defaultValue"]) {
				struct_element.DefaultValue = res;
			}

			shader.ShaderParameters.EmplaceBack(struct_element);
		}
	}

	if(auto tr = json[u8"transparency"]) {
		shader.Transparency = tr.GetBool();
	}

	if (auto fs = json[u8"functions"]) {
		for (auto f : fs) {
			auto& fd = shader.Functions.EmplaceBack();

			fd.Return = f[u8"return"];
			fd.Name = f[u8"name"];

			for (auto p : f[u8"params"]) {
				fd.Parameters.EmplaceBack(p[u8"type"], p[u8"name"]);
			}

			for (auto s : f[u8"statements"]) {
				auto& st = fd.Statements.EmplaceBack(BE::PAR(u8"ShaderGenerator"));
				parseStatement(s, st, 0);
			}
		}
	}

	{
		for (auto e : json[u8"statements"]) {
			shader.statements.EmplaceBack(BE::PAR(u8"ShaderGenerator"));
			parseStatement(e, shader.statements.back(), 0);
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

		if (GTSL::ModuloByPowerOf2(string.GetBytes(), 4) == 0) { //if all non null terminator characters are a multiple of 4 bytes that means that all groups of four bytes where put in an int and no free byte was left to represent a null terminator
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

		if (debugMode) {
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

		for (auto& e : params) {
			addInst(spv::OpFunctionParameter);
		}

		addInst(spv::OpReturn);
		addInst(spv::OpFunctionEnd);
	};
}