#pragma once

#include <GTSL/String.hpp>
#include "ByteEngine/Application/AllocatorReferences.h"
#include <GAL/RenderCore.h>
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
		ID, OP, LITERAL, LPAREN, RPAREN, COMMA
	} ValueType;

	GTSL::StaticString<64> Name;

	ShaderNode() = default;
	ShaderNode(Type t, const GTSL::StringView na) : ValueType(t), Name(na) {}

	auto GetName() const -> GTSL::StringView {
		return Name;
	}
};

bool IsAnyOf(const auto& a, const auto&... elems) {
	return ((a == elems) or ...);
}

//inline auto parseStatement(GTSL::JSONMember parent, GTSL::Tree<ShaderNode, BE::PAR>& tree, uint32 parentHandle) -> uint32 {
//	auto handle = tree.Emplace(parentHandle);
//	auto& node = tree[handle];
//
//	if (auto nameMember = parent[u8"name"]) { //var || var decl || func || operator
//		node.Name = GTSL::StringView(nameMember);
//
//		if (auto paramsMember = parent[u8"params"]) { //function, var decl
//			if (auto typeMember = parent[u8"type"]) { //name ^ params ^ type -> var decl
//				//node.ValueType = ShaderNode::Type::VAR_DEC;
//				//node.TypeName = GTSL::StringView(typeMember);
//			}
//			else { //name ^ params ^ ~type -> function
//				if (GTSL::IsSymbol(nameMember.GetStringView()[0])) {
//					node.ValueType = ShaderNode::Type::OPERATOR;
//				}
//				else if (nameMember.GetStringView() == u8"return") {
//					node.ValueType = ShaderNode::Type::RETURN;
//				}
//				else {
//					node.ValueType = ShaderNode::Type::FUNCTION;
//				}
//			}
//
//			for (auto e : parent[u8"params"]) {
//				parseStatement(e, tree, handle);
//			}
//		}
//		else { //name and no params -> var
//			node.ValueType = ShaderNode::Type::VARIABLE;
//		}
//	}
//	else if (auto outputMember = parent[u8"output"]) {
//		node.Name = outputMember;
//		//node.ValueType = ShaderNode::Type::SHADER_RESULT;
//		for (auto e : parent[u8"params"]) {
//			parseStatement(e, tree, handle);
//		}
//	}
//	else { //no name -> literal
//		if (auto valueMember = parent[u8"value"]) {
//			node.Name = valueMember;
//			node.ValueType = ShaderNode::Type::LITERAL;
//		}
//		else {
//			//node.TypeName = parent[u8"type"];
//			node.ValueType = ShaderNode::Type::RVALUE;
//			for (auto e : parent[u8"params"]) {
//				parseStatement(e, tree, handle);
//			}
//		}
//	}
//
//	return handle;
//}

struct Shader {
	enum class Class { VERTEX, SURFACE, COMPUTE, RENDER_PASS, RAY_GEN, MISS };

	Shader(const GTSL::StringView name, const Class clss) : Name(name), Type(clss) {

	}

	void AddShaderParameter(const StructElement element) { ShaderParameters.EmplaceBack(element); }

	void SetThreadSize(const GTSL::Extent3D size) { threadSize = size; }

	GTSL::ShortString<32> Name;
	Class Type;

	GTSL::StaticVector<StructElement, 8> ShaderParameters;

	//compute
	GTSL::Extent3D threadSize;

	bool Transparency = false;

	struct FunctionDefinition {
		GTSL::StaticString<32> Return, Name;
		GTSL::StaticVector<StructElement, 8> Parameters;
		GTSL::StaticVector<GTSL::StaticVector<ShaderNode, 32>, 8> Statements;
	};
	GTSL::StaticVector<FunctionDefinition, 8> Functions;
};

struct GPipeline {
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
		GTSL::StaticString<512> Code;
		GTSL::StaticVector<GTSL::StaticVector<ShaderNode, 32>, 8> Statements;
		bool IsRaw = false, Inline = false;

		//Every function gets assigned an id which is unique per pipeline
		//It aides in identifying functions when dealing with overloads, which share name and thus does not allow to uniquely identify them
		//Id can also be used to access the element which represents this function
		uint32 Id = 0;
	};
	GTSL::Vector<FunctionDefinition, BE::TAR> Functions;

	GTSL::ShortString<32> TargetSemantics;

	GAL::IndexType IndexType = GAL::IndexType::UINT16;

	struct LanguageElement {
		LanguageElement(const BE::TAR& allocator) : map(16, allocator) {}

		enum class Type {
			NULL, SCOPE, KEYWORD, TYPE, MEMBER, FUNCTION
		} T = Type::NULL;

		GTSL::HashMap<Id, GTSL::StaticVector<uint32, 8>, BE::TAR> map;

		uint32 Reference = 0xFFFFFFFF;
	};
	GTSL::Tree<LanguageElement, BE::TAR> elements;

	struct ElementHandle { uint32 Handle = 1; };

	ElementHandle VertexShaderScope, FragmentShaderScope, ComputeShaderScope, RayGenShaderScope, ClosestHitShaderScope, MissShaderScope;

	GTSL::StaticVector<GTSL::StaticString<64>, 16> DS;

	GPipeline() : elements(32, BE::TAR(u8"Shader")), Functions(BE::TAR(u8"Shader")) {
		elements.Emplace(0, BE::TAR(u8"Shader"));

		Add(ElementHandle(), u8"=", LanguageElement::Type::FUNCTION);
		Add(ElementHandle(), u8"*", LanguageElement::Type::FUNCTION);
		Add(ElementHandle(), u8"return", LanguageElement::Type::KEYWORD);
		Add(ElementHandle(), u8"uint32", LanguageElement::Type::TYPE);
		Add(ElementHandle(), u8"float32", LanguageElement::Type::TYPE);
		Add(ElementHandle(), u8"vec2f", LanguageElement::Type::TYPE);
		Add(ElementHandle(), u8"vec3f", LanguageElement::Type::TYPE);
		Add(ElementHandle(), u8"vec4f", LanguageElement::Type::TYPE);
	}

	auto& GetElement(ElementHandle parent, const GTSL::StringView name) {
		return elements[elements[parent.Handle].map[Id(name)].back()];
	}

	auto& GetElement(const GTSL::Range<const ElementHandle*> parents, const GTSL::StringView name) {
		for (auto& p : parents) {
			if (auto res = TryGetElement(p, name)) {
				return res.Get();
			}
		}

		return elements[0];
	}

	ElementHandle Add(ElementHandle parent, const GTSL::StringView name, LanguageElement::Type type) {
		auto handle = elements.Emplace(parent.Handle, BE::TAR(u8"Shader"));
		elements[parent.Handle].map.Emplace(Id(name)).EmplaceBack(handle);
		auto& e = elements[handle];
		e.T = type;
		return ElementHandle(handle);
	}

	ElementHandle addConditional(ElementHandle parent, const GTSL::StringView name, LanguageElement::Type type) {
		auto handle = elements.Emplace(parent.Handle, BE::TAR(u8"Shader"));
		elements[parent.Handle].map.TryEmplace(Id(name)).Get().EmplaceBack(handle);
		auto& e = elements[handle];
		e.T = type;
		return ElementHandle(handle);
	}

	auto TryGetElement(ElementHandle parent, const GTSL::StringView name) -> GTSL::Result<LanguageElement&> {
		if (auto res = elements[parent.Handle].map.TryGet(Id(name))) {
			return { elements[res.Get().back()], true };
		} else {
			return { elements[0], false };
		}
	}

	auto TryGetElement(const GTSL::Range<const ElementHandle*> parents, const GTSL::StringView name) -> GTSL::Result<LanguageElement&> {
		for (auto& p : parents) {
			if (auto res = TryGetElement(p, name)) {
				return res;
			}
		}

		return { elements[0], false };
	}

	auto& DeclareVertexElement(GAL::Pipeline::VertexElement vertex_element) {
		GTSL::StaticString<64> name(u8"in_"); name += vertex_element.Identifier;
		auto handle = Add({}, name, LanguageElement::Type::MEMBER);
		return VertexElements.EmplaceBack(vertex_element);
	}

	auto& DeclareFunction(ElementHandle parent, const GTSL::StringView returnType, const GTSL::StringView name) {
		auto handle = addConditional(parent, name, LanguageElement::Type::FUNCTION);
		elements[handle.Handle].Reference = Functions.GetLength();
		auto& function = Functions.EmplaceBack();
		function.Name = name;
		function.Return = returnType;
		function.Id = handle.Handle;
		return function;
	}

	auto& DeclareFunction(ElementHandle parent, const GTSL::StringView returnType, const GTSL::StringView name, const GTSL::Range<const StructElement*> parameters, const GTSL::StringView code) {
		auto handle = addConditional(parent, name, LanguageElement::Type::FUNCTION);
		elements[handle.Handle].Reference = Functions.GetLength();
		auto& function = Functions.EmplaceBack();
		function.Name = name;
		function.Return = returnType;
		function.Parameters = parameters;
		function.Code = code;
		function.Id = handle.Handle;
		return function;
	}

	auto& DeclareRawFunction(ElementHandle parent, const GTSL::StringView returnType, const GTSL::StringView name, const GTSL::Range<const StructElement*> parameters, const GTSL::StringView code) {
		auto handle = addConditional(parent, name, LanguageElement::Type::FUNCTION);
		elements[handle.Handle].Reference = Functions.GetLength();
		auto& function = Functions.EmplaceBack();
		function.Name = name;
		function.Return = returnType;
		function.Parameters = parameters;
		function.Code = code;
		function.IsRaw = true;
		function.Id = handle.Handle;
		return function;
	}

	auto& GetFunction(uint32 id) {
		return Functions[elements[id].Reference];
	}

	auto& GetFunction(GTSL::Range<const ElementHandle*> parents, const GTSL::StringView name) {
		for (auto& p : parents) {
			if (auto res = TryGetElement(p, name)) {
				return Functions[res.Get().Reference];
			}
		}

		return Functions[0];
	}

	auto GetFunctionOverloads(GTSL::Range<const ElementHandle*> parents, const GTSL::StringView name) {
		for (auto& p : parents) {
			if (auto res = TryGetElement(p, name)) {
				GTSL::StaticVector<FunctionDefinition*, 8> overloads;

				for (auto& e : elements[p.Handle].map[name]) {
					overloads.EmplaceBack(&Functions[elements[e].Reference]);
				}

				return overloads;
			}
		}

		return GTSL::StaticVector<FunctionDefinition*, 8>();
	}

	auto GetVertexElements() const { return VertexElements.GetRange(); }

	void DeclareStruct(const ElementHandle parent, const GTSL::StringView name, GTSL::Range<const StructElement*> members) {
		auto handle = Add(parent, name, LanguageElement::Type::TYPE);

		for(auto& e : members) {
			Add(handle, e.Name, LanguageElement::Type::MEMBER);
		}
	}

	void DeclareVariable(const ElementHandle parentHandle, const GTSL::StringView interfaceName, const GTSL::StringView fullString) {
		auto handle = Add(parentHandle, interfaceName, LanguageElement::Type::MEMBER);
		elements[handle.Handle].Reference = DS.GetLength();
		DS.EmplaceBack(fullString);
	}

	GTSL::StringView GetDS(const GTSL::Range<const ElementHandle*> element_handles, const GTSL::StringView name) {
		for(uint32 i = 0, j = element_handles.ElementCount() - 1; i < element_handles.ElementCount(); ++i, --j) {
			if (auto res = TryGetElement(element_handles[j], name)) { return DS[res.Get().Reference]; }
		}

		return {};
	}

private:
	GTSL::StaticVector<GAL::Pipeline::VertexElement, 32> VertexElements;
};

inline void parseCode(const GTSL::StringView code, GPipeline& pipeline, auto& statements, const GTSL::Range<const GPipeline::ElementHandle*> scopes) {
	enum class TokenTypes { ID, OP, NUM, LPAREN, RPAREN, COMMA, END };
	GTSL::StaticVector<GTSL::StaticString<64>, 128> tokens;
	GTSL::StaticVector<TokenTypes, 128> tokenTypes;

	auto codeString = code;

	for (uint32 i = 0; i < code.GetCodepoints(); ++i) {
		auto c = code[i];

		TokenTypes type;

		if (GTSL::IsWhitespace(c)) { continue; }

		GTSL::StaticString<64> str;

		if (GTSL::IsSymbol(c) and c != U'.' and c != U'_') {
			if (c == U'(') {
				type = TokenTypes::LPAREN;
			} else if (c == U')') {
				type = TokenTypes::RPAREN;
			} else if (c == U',') {
				type = TokenTypes::COMMA;
			} else if (c == U';') {
				type = TokenTypes::END;
			} else if (IsAnyOf(c, U'=', U'*')) {
				type = TokenTypes::OP;
			}

			str += c;

			tokens.EmplaceBack(str);
			tokenTypes.EmplaceBack(type);
		} else {
			while(GTSL::IsLetter(code[i]) or GTSL::IsNumber(code[i]) or code[i] == U'.' or code[i] == U'_') {
				str += code[i];
				++i;
			}

			if (IsNumber(str)) {
				type = TokenTypes::NUM;
			} else {
				type = TokenTypes::ID;
			}

			tokens.EmplaceBack(str);
			tokenTypes.EmplaceBack(type);

			--i;
		}

	}

	for (uint32 i = 0, s = 0; i < tokens; ++i) {
		if (tokenTypes[i] != TokenTypes::END) { continue; }

		auto& statement = statements.EmplaceBack();

		for (uint32 j = s; j < i; ++j) {
			if (tokenTypes[j] == TokenTypes::ID or tokenTypes[j] == TokenTypes::OP) {
				const auto& element = pipeline.GetElement(scopes, tokens[j]);

				if (tokenTypes[j] == TokenTypes::ID) {
					statement.EmplaceBack(ShaderNode::Type::ID, tokens[j]);
				}
				else {
					statement.EmplaceBack(ShaderNode::Type::OP, tokens[j]);
				}
			}
			else {
				ShaderNode::Type type;

				switch (tokenTypes[j]) {
				case TokenTypes::NUM: type = ShaderNode::Type::LITERAL; break;
				case TokenTypes::LPAREN: type = ShaderNode::Type::LPAREN; break;
				case TokenTypes::RPAREN: type = ShaderNode::Type::RPAREN; break;
				case TokenTypes::COMMA: type = ShaderNode::Type::COMMA; break;
				case TokenTypes::END: break;
				}

				statement.EmplaceBack(type, tokens[j]);
			}
		}

		s = i + 1;
	}
}

inline GTSL::StaticString<8192> GenerateShader(Shader& shader, GPipeline& pipeline, GPipeline::ElementHandle scope, GAL::ShaderType targetSemantics) {
	GTSL::StaticString<2048> headerBlock, structBlock, functionBlock, declarationBlock;

	headerBlock += u8"#version 460 core\n"; //push version

	switch (Hash(pipeline.TargetSemantics)) {
	case GTSL::Hash(u8"raster"): {
	}
	case GTSL::Hash(u8"compute"): {
	}
	case GTSL::Hash(u8"rayTrace"):
		headerBlock += u8"#extension GL_EXT_ray_tracing : enable\n";
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

	struct VariableDeclaration {
		StructElement Element;
	};
	GTSL::HashMap<Id, VariableDeclaration, GTSL::DefaultAllocatorReference> variables(16, 1.0f);

	auto resolveTypeName = [&](const GTSL::StringView name) -> GTSL::StaticString<64> {
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
			GTSL::StaticString<64> n(name);
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

	auto declareStruct = [&](GTSL::StringView ne, GTSL::Range<const StructElement*> structElements, bool ref, bool readOnly = true) {
		GTSL::StaticString<32> name(ne);

		if (ref) { name += u8"Pointer"; }

		GTSL::StaticVector<StructElement, 16> stt;

		if (!pipeline.TryGetElement(scope, name)) {
			pipeline.DeclareStruct(scope, name, structElements);
		}

		if (!structElements.ElementCount()) {
			stt.EmplaceBack(u8"uint32", u8"dummy");
		} else {
			stt.PushBack(structElements);
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

		for (auto& e : stt) {
			structBlock += resolveTypeName(e.Type); structBlock += u8' '; structBlock += e.Name; structBlock += u8"; ";
		}

		structBlock += u8"};\n";
	};

	GTSL::StaticMap<uint32, bool, 32> usedFunctions(16); //TODO: try emplace return is a reference which might be invalidated if map is resized during recursive call

	auto useFunction = [&pipeline, scope, &usedFunctions, resolveTypeName](auto& resultString, const uint32 id, auto&& self) -> void {
		auto& function = pipeline.GetFunction(id);

		auto functionUsed = usedFunctions.TryEmplace(function.Id, false);

		GTSL::StaticString<512> string;

		if (!functionUsed.Get()) {
			string += resolveTypeName(function.Return); string += u8' ';  string += function.Name;

			string += u8"(";

			uint32 paramCount = function.Parameters.GetLength();

			for (uint32 i = 0; i < paramCount; ++i) {
				string += resolveTypeName(function.Parameters[i].Type); string += u8' '; string += function.Parameters[i].Name;
				if (i != paramCount - 1) { string += u8", "; }
			}

			string += u8") { ";

			if(!function.IsRaw) {
				if (!function.Statements) {
					parseCode(function.Code, pipeline, function.Statements, { {}, scope });
				}

				for(auto& s : function.Statements) {
					for(auto& node : s) {
						switch (node.ValueType) {
						case ShaderNode::Type::ID: {
							auto element = pipeline.GetElement({ {}, scope }, node.Name);

							if (element.T == GPipeline::LanguageElement::Type::MEMBER) {
								auto r = pipeline.GetDS({ {}, scope }, node.Name);

								if (r.GetBytes()) {
									string += r;
								}
							}
							else {
								if (auto res = pipeline.TryGetElement({ {}, scope }, node.Name)) {
									if (res.Get().T == GPipeline::LanguageElement::Type::FUNCTION) {
										for (auto e : pipeline.GetFunctionOverloads({ {}, scope }, node.Name)) { //for every overload, TODO: type deduction?
											self(resultString, e->Id, self); //add function
										}
									}
								}

								if (node.Name == u8"return") {
									string += u8"return ";
								} else {
									string += resolveTypeName(node.Name);
								}
							}

							break;
						}
						case ShaderNode::Type::LPAREN: {
							string += u8"(";
							break;
						}
						case ShaderNode::Type::RPAREN: {
							string += u8")";
							break;
						}
						case ShaderNode::Type::LITERAL: {
							string += node.Name;
							break;
						}
						case ShaderNode::Type::OP: {
							string += u8' '; string += node.Name; string += u8' ';
							break;
						}
						case ShaderNode::Type::COMMA: {
							string += u8", ";
							break;
						}
						}
					}

					string += u8"; ";
				}
			} else {
				string += function.Code;
			}

			string += u8" }\n";

			functionUsed.Get() = true;
		}

		resultString += string;
	};

	//using TTT = decltype(static_cast<const GTSL::Tree<ShaderNode, BE::PAR>&>(shader.statements[0]).begin());

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

	for (const auto& e : pipeline.GetVertexElements()) {
		vertexElements.EmplaceBack(shaderDataTypeToType(e.Type), e.Identifier);
	}

	declareStruct(u8"vertex", vertexElements, false);
	declareStruct(u8"vertex", vertexElements, true);

	declareStruct(u8"index", { { u8"uint16", u8"i" } }, true);

	for (const auto& s : pipeline.Structs) {
		declareStruct(s.Name, s.Members, false);
		declareStruct(s.Name, s.Members, true);
	}

	declareStruct(u8"shaderParametersData", pipeline.parameters, true);

	{ //push constant
		declarationBlock += u8"layout(push_constant, scalar) uniform _invocationInfo { ";

		addVariable(u8"invocationInfo", { u8"push_constant", u8"invocationInfo" });

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
			declarationBlock += targetSemantics == GAL::ShaderType::RAY_GEN ? u8"rayPayloadEXT " : u8"rayPayloadInEXT ";
			declarationBlock += resolveTypeName(e.Type); declarationBlock += e.Name; declarationBlock += u8";\n";
			++i;
		}
	}
	}

	auto addShaderRecordDeclaration = [&](const uint8 index) {
		declarationBlock += u8"layout(shaderRecordEXT, scalar) buffer shader { ";

		addVariable(u8"shaderRecordEXT", { u8"shader", u8"invocationInfo" });

		for (auto& e : pipeline.ShaderRecord[index]) {
			addStructElement(declarationBlock, e);
		}

		declarationBlock += u8" };\n";
	};

	switch (targetSemantics) {
	case GAL::ShaderType::VERTEX: {
		declarationBlock += u8"layout(location=0) out vertexData { ";
		for (auto& e : pipeline.Interface) {
			addStructElement(declarationBlock, e);
		}
		declarationBlock += u8" } vertexOut;\n";

		for (uint8 i = 0; const auto & ve : pipeline.GetVertexElements()) {
			GTSL::StaticString<64> name(u8"in_"); name += ve.Identifier;

			declarationBlock += u8"layout(location="; ToString(declarationBlock, i); declarationBlock += u8") in ";
			declarationBlock += resolveTypeName(shaderDataTypeToType(ve.Type));
			declarationBlock += u8' '; declarationBlock += name; declarationBlock += u8";\n";
			++i;
			addVariable(name, { shaderDataTypeToType(ve.Type), name });
		}

		break;
	}
	case GAL::ShaderType::MESH:
		declarationBlock += u8"layout(local_size_x="; ToString(declarationBlock, 32); declarationBlock += u8") in;\n";
		declarationBlock += u8"layout(triangles) out;\n";
		declarationBlock += u8"layout(max_vertices=64, max_primitives=126) out;\n";
		break;
	case GAL::ShaderType::CLOSEST_HIT:
		declarationBlock += u8"hitAttributeEXT vec2 hitBarycenter;\n";
		break;
	case GAL::ShaderType::ANY_HIT:
		break;
	case GAL::ShaderType::INTERSECTION: {
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

		for (uint32 i = 0; auto & o : pipeline.Outputs) {
			GTSL::StaticString<64> name(u8"out_"); name += o.Name;

			declarationBlock += u8"layout(location="; ToString(declarationBlock, i); declarationBlock += u8") out ";
			addStructElement(declarationBlock, { o.Type, name, o.DefaultValue }); declarationBlock += u8'\n';
			++i;

			addVariable(name, o);
		}

		break;
	}
	case GAL::ShaderType::COMPUTE: {
		GTSL::Extent3D size = shader.threadSize;
		declarationBlock += u8"layout(local_size_x="; ToString(declarationBlock, size.Width);
		declarationBlock += u8",local_size_y="; ToString(declarationBlock, size.Height);
		declarationBlock += u8",local_size_z="; ToString(declarationBlock, size.Depth);
		declarationBlock += u8") in;\n";

		break;
	}
	case GAL::ShaderType::TASK:
		break;
	case GAL::ShaderType::RAY_GEN: {
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

	useFunction(functionBlock, pipeline.GetFunction({ {}, scope }, u8"main").Id, useFunction);

	GTSL::StaticString<8192> fin;

	fin += headerBlock;
	fin += structBlock;
	fin += declarationBlock;
	fin += functionBlock;

	return fin;
}

// SHADER DOC
// Class: Could be thought of as shader use, (Surface, Vertex, PostProcess, RayGen, Miss, Etc)
// TargetSemantics: target shader stage, (Vertex, Fragment, ClosestHit, AnyHit, Miss, Compute, Etc)
// GPipeline: defines environment for shader to operate in. Defines how common data is accessed so that the shader generator knows how to
// seamlessly translate Classes to TargetSemantics

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

//GTSL::StaticVector<uint32, 16> stack, queue;
//
//auto infixToPostfix = [&] {
//	while (true) {
//		const auto& token = tokens[i]; const auto tokenType = tokenTypes[i];
//		auto& statement = shader.statements.back();
//
//		uint32 currentIndex = i;
//
//		++i;
//
//		switch (tokenType) {
//		case TokenTypes::ID:
//			queue.EmplaceBack(currentIndex);
//			break;
//		case TokenTypes::OP:
//
//			if (stack) {
//				if (PRECEDENCE[Id(tokens[stack.back()])] > PRECEDENCE[Id(token)]) {
//					queue.EmplaceBack(stack.back());
//					stack.PopBack();
//				}
//			}
//
//			stack.EmplaceBack(currentIndex);
//			break;
//		case TokenTypes::NUM:
//			queue.EmplaceBack(currentIndex);
//			break;
//		case TokenTypes::LPAREN:
//			stack.EmplaceBack(currentIndex);
//			break;
//		case TokenTypes::RPAREN:
//			while(tokenTypes[stack.back()] != TokenTypes::LPAREN) {
//				queue.EmplaceBack(stack.back());
//				stack.PopBack();
//			}
//
//			stack.PopBack();
//			break;
//		case TokenTypes::COMMA:
//			break;
//		case TokenTypes::END: return;
//		default: ;
//		}
//
//	}
//};
//
//shader.statements.EmplaceBack(BE::PAR(u8"ShaderGenerator"));
//infixToPostfix();
//
//while(stack) { //transfer all remaining map
//	queue.EmplaceBack(stack.back());
//	stack.PopBack();
//}

//if (tokenTypes[i] != TokenTypes::END) { continue; }
//
//auto& statement = shader.statements.EmplaceBack(BE::PAR(u8"ShaderGenerator"));
//
//auto evalExpression = [&](uint32 start, uint32 p, uint32 parentHandle, uint32* l, auto&& self) -> uint32 {
//	if (tokenTypes[start] == TokenTypes::LPAREN) { ++*l; return p + 1; }
//	if (tokenTypes[start] == TokenTypes::RPAREN) { ++*l; return p - 1; }
//	if (tokenTypes[start] == TokenTypes::COMMA) { ++*l; return p; }
//
//	uint32 minPrecedence = 0xFFFFFFFF, index = 0;
//
//	for (uint32 x = start; x < i; ++x) {
//		auto currentTokenType = tokenTypes[x];
//
//		if (auto precedence = getPrecedence(tokens[x]); precedence < minPrecedence) {
//			minPrecedence = precedence;
//			index = x;
//		}
//	}
//
//	bool isOperator = IsAnyOf(tokens[index], u8"=");
//	uint32 handle = 0;
//	const auto& element = shader.GetElement(Shader::ElementHandle(), tokens[index]);
//
//	switch (element.T) {
//	break; case Shader::LanguageElement::Type::TYPE: handle = statement.Emplace(parentHandle, ShaderNode::Type::RVALUE, tokens[index]);
//	break; case Shader::LanguageElement::Type::MEMBER: handle = statement.Emplace(parentHandle, ShaderNode::Type::VARIABLE, tokens[index]);
//	break; case Shader::LanguageElement::Type::FUNCTION:
//		handle = statement.Emplace(parentHandle, isOperator ? ShaderNode::Type::OPERATOR : ShaderNode::Type::FUNCTION, tokens[index]);
//	break; default:;
//	}
//
//	if (element.T == Shader::LanguageElement::Type::FUNCTION) {
//		if (isOperator) {
//			std::swap(tokens[index], tokens[index - 1]);
//			std::swap(tokenTypes[index], tokenTypes[index - 1]);
//			index -= 1;
//			++p;
//		}
//
//		uint32 t = 0;
//
//		while (true) {
//			if (self(index + 1 + t, p + 1, handle, &t, self) == p) { break; }
//		}
//	}
//
//	++* l;
//
//	return p;
//};
//
//uint32 l = 0;
//evalExpression(lastStatement, 0, 0, &l, evalExpression);
//lastStatement = i;

//GTSL::StaticMap<Id, uint8, 16> PRECEDENCE(16);
//PRECEDENCE.Emplace(u8"=", 1);
//PRECEDENCE.Emplace(u8"||", 2);
//PRECEDENCE.Emplace(u8"<", 7); PRECEDENCE.Emplace(u8">", 7); PRECEDENCE.Emplace(u8"<=", 7); PRECEDENCE.Emplace(u8">=", 7); PRECEDENCE.Emplace(u8"==", 7); PRECEDENCE.Emplace(u8"!=", 7);
//PRECEDENCE.Emplace(u8"+", 10); PRECEDENCE.Emplace(u8"-", 10);
//PRECEDENCE.Emplace(u8"*", 20); PRECEDENCE.Emplace(u8"/", 20); PRECEDENCE.Emplace(u8"%", 20);
//
//auto getPrecedence = [&](const GTSL::StringView name) -> uint32 {
//	if (auto pre = PRECEDENCE.TryGet(Id(name))) {
//		return pre.Get();
//	}
//	else {
//		return 30;
//	}
//};