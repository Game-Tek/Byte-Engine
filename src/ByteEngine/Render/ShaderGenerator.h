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
	StructElement() = default;
	StructElement(const GTSL::StringView t, const GTSL::StringView n) : Type(t), Name(n) {}
	StructElement(const GTSL::StringView t, const GTSL::StringView n, const GTSL::StringView dv) : Type(t), Name(n), DefaultValue(dv) {}

	GTSL::StaticString<64> Type, Name, DefaultValue;
};

struct ShaderNode {
	enum class Type : uint8 {
		NULL, ID, OP, LITERAL, LPAREN, RPAREN, LBRACKET, RBRACKET, LBRACE, RBRACE, DOT, COMMA, SEMICOLON
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

enum class Class { VERTEX, SURFACE, COMPUTE, RENDER_PASS, RAY_GEN, MISS };

/**
 * \brief Turns code into a stream of tokens, every first dimension is an statement, all elements in the array's second dimension is a token. Can only parse a functions content, no language constructs (classes, enums, descriptors, etc...)
 * \param code String containing code to tokenize.
 * \param statements Array container for statements.
 */
void tokenizeCode(const GTSL::StringView code, auto& statements, const auto& allocator) {
	tokenizeCode(code, statements);
}

void tokenizeCode(const GTSL::StringView code, auto& statements) {
	GTSL::StaticVector<GTSL::StaticString<64>, 1024> tokens;
	GTSL::StaticVector<ShaderNode::Type, 1024> tokenTypes;

	auto codeString = code;

	for (uint32 i = 0; i < code.GetCodepoints(); ++i) {
		auto c = code[i];

		ShaderNode::Type type;

		GTSL::StaticString<64> str;

		if (GTSL::IsSymbol(c)) {
			if (c == U'(') {
				type = ShaderNode::Type::LPAREN;
			}
			else if (c == U')') {
				type = ShaderNode::Type::RPAREN;
			}
			else if (c == U'[') {
				type = ShaderNode::Type::LBRACKET;
			}
			else if (c == U']') {
				type = ShaderNode::Type::RBRACKET;
			}
			else if (c == U'{') {
				type = ShaderNode::Type::LBRACE;
			}
			else if (c == U'}') {
				type = ShaderNode::Type::RBRACE;
			}
			else if (c == U'.') {
				type = ShaderNode::Type::DOT;
			}
			else if (c == U',') {
				type = ShaderNode::Type::COMMA;
			}
			else if (c == U';') {
				type = ShaderNode::Type::SEMICOLON;
			}
			else if (IsAnyOf(c, U'=', U'*', U'+', U'-', U'/', U'%')) {
				type = ShaderNode::Type::OP;
			}

			str += c;

			tokens.EmplaceBack(str);
			tokenTypes.EmplaceBack(type);
		}
		else if (GTSL::IsNumber(c)) {
			while (GTSL::IsLetter(code[i]) or GTSL::IsNumber(code[i]) or code[i] == U'.') {
				str += code[i];
				++i;
			}

			type = ShaderNode::Type::LITERAL;

			tokens.EmplaceBack(str);
			tokenTypes.EmplaceBack(type);

			--i;
		}
		else if (GTSL::IsLetter(code[i])) {
			while (GTSL::IsLetter(code[i]) or GTSL::IsNumber(code[i]) or code[i] == U'_') {
				str += code[i];
				++i;
			}

			if (code[i] == U'*') { str += U'*'; ++i; }

			type = ShaderNode::Type::ID;

			tokens.EmplaceBack(str);
			tokenTypes.EmplaceBack(type);

			--i;
		}

		//anything else, new line, null, space, skip
	}

	for(uint32 i = 0; i < tokens; ++i) {
		statements.EmplaceBack(tokenTypes[i], tokens[i]);
	}
}

struct GPipeline : Object {
	struct ElementHandle { uint32 Handle = 1; };

	struct FunctionDefinition {
		FunctionDefinition(const BE::PAR& allocator) : Tokens(16, allocator) {}

		GTSL::StaticString<32> Return, Name;
		GTSL::StaticVector<StructElement, 12> Parameters;
		//GTSL::StaticString<512> Code;
		GTSL::Vector<ShaderNode, BE::PAR> Tokens;
		bool IsRaw = false, Inline = false;

		//Every function gets assigned an id which is unique per pipeline
		//It aides in identifying functions when dealing with overloads, which share name and thus does not allow to uniquely identify them
		//Id can also be used to access the element which represents this function
		uint32 Id = 0;
	};

	struct LanguageElement {
		LanguageElement(const BE::TAR& allocator) : map(16, allocator) {}

		enum class ElementType {
			NULL, MODEL, SCOPE, KEYWORD, TYPE, STRUCT, MEMBER, FUNCTION, DEDUCTION_GUIDE, DISABLED
		} Type = ElementType::NULL;

		GTSL::HashMap<Id, GTSL::StaticVector<uint32, 8>, BE::TAR> map;
		GTSL::StaticVector<uint32, 32> symbols;
		GTSL::StaticString<64> Name;
		uint16 Level = 0;
		uint32 Reference = 0xFFFFFFFF;
	};

	GPipeline() : elements(32, BE::TAR(u8"Shader")), members(32, BE::TAR(u8"Shader")), Functions(32, BE::TAR(u8"Shader")), deductionGuides(16, BE::TAR(u8"Shader")) {
		auto handle = elements.Emplace(0, BE::TAR(u8"Shader"));
		auto& e = elements[handle];
		e.Type = LanguageElement::ElementType::SCOPE;
		e.Name = u8"global";

		Add(ElementHandle(), u8"=", LanguageElement::ElementType::FUNCTION);
		Add(ElementHandle(), u8"+", LanguageElement::ElementType::FUNCTION);
		Add(ElementHandle(), u8"-", LanguageElement::ElementType::FUNCTION);
		Add(ElementHandle(), u8"*", LanguageElement::ElementType::FUNCTION);
		Add(ElementHandle(), u8"/", LanguageElement::ElementType::FUNCTION);
		Add(ElementHandle(), u8"return", LanguageElement::ElementType::KEYWORD);
		Add(ElementHandle(), u8"uint32", LanguageElement::ElementType::TYPE);
		Add(ElementHandle(), u8"float32", LanguageElement::ElementType::TYPE);
		Add(ElementHandle(), u8"vec2f", LanguageElement::ElementType::TYPE);
		Add(ElementHandle(), u8"vec3f", LanguageElement::ElementType::TYPE);
		Add(ElementHandle(), u8"vec4f", LanguageElement::ElementType::TYPE);
		Add(ElementHandle(), u8"vec2u", LanguageElement::ElementType::TYPE);
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

		__debugbreak();
	}

	uint32 GetLevel(const ElementHandle element_handle) const { return elements[element_handle.Handle].Level; }

	auto& GetElement(const ElementHandle element_handle) { return elements[element_handle.Handle]; }
	const auto& GetElement(const ElementHandle element_handle) const { return elements[element_handle.Handle]; }

	ElementHandle Add(ElementHandle parent, const GTSL::StringView name, LanguageElement::ElementType type) {
		auto handle = elements.Emplace(parent.Handle, BE::TAR(u8"Shader"));
		elements[parent.Handle].map.Emplace(Id(name)).EmplaceBack(handle);
		elements[parent.Handle].symbols.EmplaceBack(handle);
		auto& e = elements[handle];
		e.Type = type; e.Name = name;
		if(parent.Handle != 1) {
			e.Level = GetElement(parent).Level + 1;
		}
		return ElementHandle(handle);
	}

	ElementHandle addConditional(ElementHandle parent, const GTSL::StringView name, LanguageElement::ElementType type) {
		auto handle = elements.Emplace(parent.Handle, BE::TAR(u8"Shader"));
		elements[parent.Handle].map.TryEmplace(Id(name)).Get().EmplaceBack(handle);
		auto& e = elements[handle];
		e.Type = type;
		if (parent.Handle != 1) {
			e.Level = GetElement(parent).Level + 1;
		}
		return ElementHandle(handle);
	}

	auto TryGetElement(ElementHandle parent, const GTSL::StringView name) -> GTSL::Result<LanguageElement&> {
		if (auto res = elements[parent.Handle].map.TryGet(Id(name))) {
			return { elements[res.Get().back()], true };
		}
		else {
			return { elements[0], false };
		}
	}

	auto TryGetElement(ElementHandle parent, const GTSL::StringView name) const -> GTSL::Result<const LanguageElement&> {
		if (auto res = elements[parent.Handle].map.TryGet(Id(name))) {
			return { elements[res.Get().back()], true };
		}
		else {
			return { elements[0], false };
		}
	}

	auto TryGetElementHandle(ElementHandle parent, const GTSL::StringView name) const -> GTSL::Result<ElementHandle> {
		if (auto res = elements[parent.Handle].map.TryGet(Id(name))) {
			return { ElementHandle(res.Get().back()), true };
		}

		return { ElementHandle(), false };
	}

	auto TryGetElement(const GTSL::Range<const ElementHandle*> parents, const GTSL::StringView name) -> GTSL::Result<LanguageElement&> {
		for (uint32 i = parents.ElementCount() - 1, j = 0; j < parents.ElementCount(); --i, ++j) {
			if (auto res = TryGetElement(parents[i], name)) {
				return res;
			}
		}

		return { elements[0], false };
	}

	auto TryGetElement(const GTSL::Range<const ElementHandle*> parents, const GTSL::StringView name) const -> GTSL::Result<const LanguageElement&> {
		for (uint32 i = parents.ElementCount() - 1, j = 0; j < parents.ElementCount(); --i, ++j) {
			if (auto res = TryGetElement(parents[i], name)) {
				return res;
			}
		}

		return { elements[0], false };
	}

	auto TryGetElementHandle(const GTSL::Range<const ElementHandle*> parents, const GTSL::StringView name) const -> GTSL::Result<ElementHandle> {
		for (uint32 i = parents.ElementCount() - 1, j = 0; j < parents.ElementCount(); --i, ++j) {
			if (auto res = TryGetElementHandle(parents[i], name)) {
				return res;
			}
		}

		return { ElementHandle(), false };
	}

	auto TryGetElementHandle(const GTSL::Range<const ElementHandle*> parents, const ElementHandle current_scope, const GTSL::StringView name) const -> GTSL::Result<ElementHandle> {
		for (uint32 i = parents.ElementCount() - 1, j = 0; j < parents.ElementCount(); --i, ++j) {
			if (auto res = TryGetElementHandle(parents[i], name); res && GetLevel(res.Get()) <= GetLevel(current_scope)) {
				return res;
			}
		}

		return { ElementHandle(), false };
	}

	auto GetChildren(const ElementHandle element_handle) {
		GTSL::StaticVector<ElementHandle, 64> children;
		for (auto& e : GetElement(element_handle).symbols) {
			children.EmplaceBack(e);
		}

		return children;
	}

	ElementHandle DeclareFunction(ElementHandle parent, const GTSL::StringView returnType, const GTSL::StringView name) {
		auto handle = addConditional(parent, name, LanguageElement::ElementType::FUNCTION);
		elements[handle.Handle].Reference = Functions.GetLength();
		auto& function = Functions.EmplaceBack(GetPersistentAllocator());
		function.Name = name; function.Return = returnType; function.Id = handle.Handle;
		return ElementHandle(elements[handle.Handle].Reference);
	}

	ElementHandle DeclareFunction(ElementHandle parent, const GTSL::StringView returnType, const GTSL::StringView name, const GTSL::Range<const StructElement*> parameters, const GTSL::StringView code) {
		auto handle = addConditional(parent, name, LanguageElement::ElementType::FUNCTION);
		elements[handle.Handle].Reference = Functions.GetLength();
		auto& function = Functions.EmplaceBack(GetPersistentAllocator());
		function.Name = name; function.Return = returnType; function.Parameters = parameters;
		function.Id = handle.Handle;
		//function.Code = code;
		tokenizeCode(code, function.Tokens, GetPersistentAllocator());
		return ElementHandle(elements[handle.Handle].Reference);
	}

	ElementHandle DeclareRawFunction(ElementHandle parent, const GTSL::StringView returnType, const GTSL::StringView name, const GTSL::Range<const StructElement*> parameters, const GTSL::StringView code) {
		auto handle = addConditional(parent, name, LanguageElement::ElementType::FUNCTION);
		elements[handle.Handle].Reference = Functions.GetLength();
		auto& function = Functions.EmplaceBack(GetPersistentAllocator());
		function.Name = name; function.Return = returnType; function.Parameters = parameters;
		//function.Code = code;
		tokenizeCode(code, function.Tokens, GetPersistentAllocator());
		function.IsRaw = true; function.Id = handle.Handle;
		return ElementHandle(elements[handle.Handle].Reference);
	}

	auto& GetFunction(uint32 id) {
		return Functions[elements[id].Reference];
	}

	auto& GetFunction(uint32 id) const {
		return Functions[elements[id].Reference];
	}

	auto& GetFunction(GTSL::Range<const ElementHandle*> parents, const GTSL::StringView name) {
		for (auto& p : parents) {
			if (auto res = TryGetElement(p, name)) {
				return Functions[res.Get().Reference];
			}
		}

		__debugbreak();
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

	ElementHandle DeclareStruct(const ElementHandle parent, const GTSL::StringView name, GTSL::Range<const StructElement*> members) {
		auto handle = Add(parent, name, LanguageElement::ElementType::STRUCT);

		for (auto& e : members) {
			DeclareVariable(handle, e);
		}

		return handle;
	}

	ElementHandle DeclareScope(const ElementHandle parentHandle, const GTSL::StringView name) {
		return Add(parentHandle, name, LanguageElement::ElementType::SCOPE);
	}

	ElementHandle DeclareVariable(const ElementHandle parentHandle, const StructElement member) {
		auto handle = Add(parentHandle, member.Name, LanguageElement::ElementType::MEMBER);
		elements[handle.Handle].Reference = members.GetLength();
		members.EmplaceBack(member);
		return { handle };
	}

	void AddMemberDeductionGuide(const ElementHandle start_cope, const GTSL::StringView interface_name, const GTSL::Range<const ElementHandle*> access_chain) {
		auto& element = GetElement(Add(start_cope, interface_name, LanguageElement::ElementType::DEDUCTION_GUIDE));
		element.Reference = deductionGuides.GetLength();
		deductionGuides.EmplaceBack().PushBack(access_chain);
	}

	GTSL::Range<const ElementHandle*> GetMemberDeductionGuide(const ElementHandle member_deduction_guide) {
		return GTSL::Range<const ElementHandle*>(deductionGuides[GetElement(member_deduction_guide).Reference]);
	}

	StructElement GetMember(ElementHandle element_handle) {
		return members[GetElement(element_handle).Reference];
	}

	GTSL::StringView GetName(const ElementHandle element_handle) const {
		return GetElement(element_handle).Name;
	}

	ElementHandle GetElementHandle(ElementHandle parent_handle, const GTSL::StringView name) const {
		return ElementHandle(elements[parent_handle.Handle].map.At(Id(name)).back());
	}
private:
	GTSL::Tree<LanguageElement, BE::TAR> elements;
	GTSL::Vector<GTSL::StaticVector<ElementHandle, 4>, BE::TAR> deductionGuides;
	GTSL::Vector<StructElement, BE::TAR> members;
	GTSL::Vector<FunctionDefinition, BE::TAR> Functions;
};

/**
 * \brief Generates a shader string from a token stream to a target shader language.
 * \tparam ALLOCATOR Allocator to allocate shader strings.
 * \param shader Shader to generate code for.
 * \param pipeline Pipeline which contains all elements needed for compilation.
 * \param scopes Scopes in which to look for symbols, precedence grows from higher positions to lower, that is if a foo() declaration exists under scope[0] and another at scope[1], scope[1].foo will be used.
 * \param targetSemantics Target shader language to generate code for.
 * \param allocator Allocator to allocate shader strings from.
 * \return Result containing an error code and two strings, one with shader code and one with all the error codes.
 */
template<class ALLOCATOR>
GTSL::Result<GTSL::Pair<GTSL::String<ALLOCATOR>, GTSL::StaticString<1024>>> GenerateShader(GPipeline& pipeline, const GTSL::Range<const GPipeline::ElementHandle*> scopes, GAL::ShaderType targetSemantics, const ALLOCATOR& allocator) {
	GTSL::String<ALLOCATOR> headerBlock(allocator), structBlock(allocator), functionBlock(allocator), declarationBlock(allocator); GTSL::StaticString<1024> errorString;

	auto addErrorCode = [&errorString](const GTSL::StringView string) {
		errorString += string; errorString += u8"\n";
	};

	headerBlock += u8"#version 460 core\n"; //push version

	bool isRayTracing = false;

	switch (targetSemantics) {
	case GAL::ShaderType::RAY_GEN:
	case GAL::ShaderType::CLOSEST_HIT:
	case GAL::ShaderType::ANY_HIT:
	case GAL::ShaderType::INTERSECTION:
	case GAL::ShaderType::CALLABLE:
	case GAL::ShaderType::MISS:
		isRayTracing = true;
	}

	headerBlock += u8"#extension GL_EXT_shader_16bit_storage : enable\n"; headerBlock += u8"#extension GL_EXT_shader_explicit_arithmetic_types_int8 : enable\n";
	headerBlock += u8"#extension GL_EXT_shader_explicit_arithmetic_types_int16 : enable\n"; headerBlock += u8"#extension GL_EXT_shader_explicit_arithmetic_types_int64 : enable\n";
	headerBlock += u8"#extension GL_EXT_nonuniform_qualifier : enable\n"; headerBlock += u8"#extension GL_EXT_scalar_block_layout : enable\n";
	headerBlock += u8"#extension GL_EXT_buffer_reference : enable\n"; headerBlock += u8"#extension GL_EXT_buffer_reference2 : enable\n";
	headerBlock += u8"#extension GL_EXT_shader_image_load_formatted : enable\n"; headerBlock += u8"#extension GL_KHR_shader_subgroup_basic : enable\n";
	headerBlock += u8"#extension GL_KHR_shader_subgroup_arithmetic  : enable\n"; headerBlock += u8"#extension GL_KHR_shader_subgroup_ballot : enable\n";
	if (isRayTracing) {
		headerBlock += u8"#extension GL_EXT_ray_tracing : enable\n";
	}
	headerBlock += u8"layout(row_major) uniform; layout(row_major) buffer;\n"; //matrix order definitions

	auto resolve = [&](const GTSL::StringView name) -> GTSL::StaticString<32> {
		GTSL::StaticString<32> result = name;

		switch (Hash(name)) {
		case GTSL::Hash(u8"float32"): result = u8"float"; break;
		case GTSL::Hash(u8"vec2f"):   result = u8"vec2"; break;
		case GTSL::Hash(u8"vec2u"):   result = u8"uvec2"; break;
		case GTSL::Hash(u8"vec3f"):   result = u8"vec3"; break;
		case GTSL::Hash(u8"vec4f"):   result = u8"vec4"; break;
		case GTSL::Hash(u8"mat2f"):   result = u8"mat2"; break;
		case GTSL::Hash(u8"mat4f"):   result = u8"mat4"; break;
		case GTSL::Hash(u8"mat3x4f"): result = u8"mat3x4"; break;
		case GTSL::Hash(u8"mat4x3f"): result = u8"mat4x3"; break;
		case GTSL::Hash(u8"uint8"):   result = u8"uint8_t"; break;
		case GTSL::Hash(u8"uint64"):  result = u8"uint64_t"; break;
		case GTSL::Hash(u8"uint32"):  result = u8"uint"; break;
		case GTSL::Hash(u8"uint16"):  result = u8"uint16_t"; break;
		case GTSL::Hash(u8"ptr_t"):   result = u8"uint64_t"; break;
		case GTSL::Hash(u8"return"):   result = u8"return "; break;
		}

		if (*(name.end() - 1) == u8'*') {
			GTSL::StaticString<64> n(name);
			DropLast(n, u8'*');
			n += u8"Pointer";
			result = n;
		}

		return result;
	};

	auto resolveTypeName = [&](const StructElement struct_element) -> StructElement {
		StructElement result = struct_element;

		if (auto res = FindFirst(struct_element.Type, U'[')) {
			result.Type.Drop(res.Get());
			auto last = FindLast(struct_element.Type, U']'); //TODO: boom no bracket pair
			for (uint32 o = res.Get(); o < last.Get() + 1; ++o) {
				result.Name += struct_element.Type[o];
			}
		}

		result.Type = resolve(result.Type);

		return result;
	};

	auto writeStructElement = [resolveTypeName](auto& string, const StructElement& element) {
		auto newName = resolveTypeName(element);
		string += newName.Type; string += u8' '; string += newName.Name; string += u8';';
	};

	[&] {
		auto descriptorSetBlockHandle = pipeline.TryGetElementHandle(scopes, u8"descriptorSetBlock");
		if (!descriptorSetBlockHandle) { addErrorCode(u8"Descriptor set block declaration was not found."); return; }

		for (uint32 s = 0; const auto & l : pipeline.GetChildren(descriptorSetBlockHandle.Get())) {
			for (uint32 ss = 0; const auto & m : pipeline.GetChildren(l)) {
				declarationBlock += u8"layout(set="; ToString(declarationBlock, s); declarationBlock += u8",binding="; ToString(declarationBlock, ss); declarationBlock += u8") uniform ";
				writeStructElement(declarationBlock, pipeline.GetMember(m)); declarationBlock += u8"\n";
				++ss;
			}
			++s;
		}
	}();

	GTSL::HashMap<uint32, bool, ALLOCATOR> usedFunctions(16, allocator);
	GTSL::HashMap<Id, bool, ALLOCATOR> usedStructs(16, allocator); //TODO: try emplace return is a reference which might be invalidated if map is resized during recursive call

	auto writeStruct = [&](GTSL::StringView ne, const GPipeline::ElementHandle structHandle, bool ref, bool readOnly, auto&& self) {
		if (usedStructs.Find(Id(ne))) { return; }

		GTSL::StaticString<32> name(ne);

		if (ref) { name += u8"Pointer"; }

		usedStructs.Emplace(Id(name), true);

		GTSL::StaticVector<StructElement, 16> stt;

		GTSL::StaticString<256> statementString;

		for (auto& e : pipeline.GetChildren(structHandle)) {
			//if (pipeline.GetElement(e).Type == GPipeline::LanguageElement::ElementType::STRUCT) {
			//	writeStruct(pipeline.GetElement(r).Name, r, true, true, writeStruct);
			//	writeStruct(pipeline.GetElement(r).Name, r, false, true, writeStruct);
			//}

			stt.EmplaceBack(pipeline.GetMember(e));
		}

		if (!stt.GetLength()) {
			stt.EmplaceBack(u8"uint32", u8"dummy");
		}

		if (ref) {
			statementString += u8"layout(buffer_reference,scalar,buffer_reference_align=2) ";

			if (readOnly)
				statementString += u8"readonly ";

			statementString += u8"buffer ";
		}
		else {
			statementString += u8"struct ";
		}

		statementString += name; statementString += u8" { ";

		for (auto& e : stt) { writeStructElement(statementString, e); }

		statementString += u8"};\n";

		structBlock += statementString;
	};

	for (auto& e : scopes) {
		for (auto& r : pipeline.GetChildren(e)) {
			if (pipeline.GetElement(r).Type == GPipeline::LanguageElement::ElementType::STRUCT) {
				writeStruct(pipeline.GetElement(r).Name, r, true, true, writeStruct);
				writeStruct(pipeline.GetElement(r).Name, r, false, true, writeStruct);
			}
		}
	}

	auto writeFunction = [&pipeline, scopes, &usedFunctions, resolveTypeName, resolve, addErrorCode, writeStruct](auto& resultString, const GPipeline::ElementHandle function_handle, const uint32 id, auto&& self) -> void {
		auto& function = pipeline.GetFunction(id);

		auto functionUsed = usedFunctions.TryEmplace(function.Id, false);

		GTSL::StaticString<2048> string;

		if (!functionUsed.Get()) {
			string += resolveTypeName({ function.Return, u8"" }).Type; string += u8' ';  string += function.Name;

			string += u8"(";

			uint32 paramCount = function.Parameters.GetLength();

			for (uint32 i = 0; i < paramCount; ++i) {
				auto param = resolveTypeName(function.Parameters[i]);
				string += param.Type; string += u8' '; string += param.Name;
				if (i != paramCount - 1) { string += u8", "; }
			}

			string += u8") { ";

			//if (!function.Statements) {
			//	tokenizeCode(function.Code, function.Statements);
			//}

			for (uint32 i = 0; i < function.Tokens;) {
				auto makeStatement = [&] {
					GTSL::StaticString<1024> statementString;

					bool isLoopBody = false;

					while (i < function.Tokens) {
						const auto& node = function.Tokens[i++];

						switch (node.ValueType) {
						case ShaderNode::Type::ID: {
							if (statementString) {
								if (GTSL::IsLetter(*(statementString.end() - 1)) || GTSL::IsNumber(*(statementString.end() - 1))) { statementString += U' '; }
							}

							//if (function.IsRaw) { statementString += resolve(node.Name); break; }

							auto elementResult = pipeline.TryGetElementHandle(scopes, function_handle, node.Name);

							if (elementResult.State()) {
								auto& element = pipeline.GetElement(elementResult.Get());

								if (element.Type == GPipeline::LanguageElement::ElementType::MEMBER) {
									statementString += node.Name;
								}
								else if (element.Type == GPipeline::LanguageElement::ElementType::DEDUCTION_GUIDE) {
									for (uint32 i = 0; auto f : pipeline.GetMemberDeductionGuide(elementResult.Get())) {
										if (i) { statementString += u8"."; }
										statementString += resolve(pipeline.GetName(f));
										++i;
									}
								}
								else if (element.Type == GPipeline::LanguageElement::ElementType::FUNCTION) {
									if (auto res = pipeline.TryGetElement(scopes, node.Name)) {
										for (auto e : pipeline.GetFunctionOverloads(scopes, node.Name)) { //for every overload, TODO: type deduction?
											self(resultString, elementResult.Get(), e->Id, self); //add function
										}
									}

									statementString += node.Name;
								} else if(element.Type == GPipeline::LanguageElement::ElementType::DISABLED) {
									return GTSL::StaticString<1024>(); //skip statement
								} else {
									statementString += resolve(node.Name);
								}
							}
							else {
								statementString += resolve(node.Name);
							}

							break;
						}
						case ShaderNode::Type::LPAREN: {
							statementString += u8"("; isLoopBody = true;
							break;
						}
						case ShaderNode::Type::RPAREN: {
							statementString += u8")"; isLoopBody = false;
							break;
						}
						case ShaderNode::Type::LBRACKET: {
							statementString += u8"[";
							break;
						}
						case ShaderNode::Type::RBRACKET: {
							statementString += u8"]";
							break;
						}
						case ShaderNode::Type::LBRACE: {
							statementString += u8"{";
							break;
						}
						case ShaderNode::Type::RBRACE: {
							statementString += u8"}";
							break;
						}
						case ShaderNode::Type::DOT: {
							statementString += u8".";
							break;
						}
						case ShaderNode::Type::LITERAL: {
							statementString += node.Name;
							break;
						}
						case ShaderNode::Type::OP: {
							//statementString += u8' '; statementString += node.Name; statementString += u8' ';
							statementString += node.Name;
							break;
						}
						case ShaderNode::Type::COMMA: {
							statementString += u8", ";
							break;
						}
						case ShaderNode::Type::SEMICOLON: {
							statementString += u8';';

						}
						}
					}

					return statementString;
				};

				string += makeStatement();
			}

			string += u8"}\n";

			functionUsed.Get() = true;
		}

		resultString += string;
	};

	declarationBlock += u8"layout(push_constant, scalar) uniform _invocationInfo { ";
	[&] { //push constant
		auto pushConstantBlockHandle = pipeline.TryGetElementHandle(scopes, u8"pushConstantBlock");
		if (!pushConstantBlockHandle) { addErrorCode(u8"Push constant block declaration was not found."); return; }
		for (const auto& l : pipeline.GetChildren(pushConstantBlockHandle.Get())) { writeStructElement(declarationBlock, pipeline.GetMember(l)); }
	}();
	declarationBlock += u8"} pushConstantBlock;\n";

	if (isRayTracing) {
		[&] {
			auto payloadBlockHandle = pipeline.TryGetElementHandle(scopes, u8"payloadBlock");
			if (!payloadBlockHandle) { addErrorCode(u8"Payload block declaration was not found."); return; }

			for (uint32 i = 0; const auto & l : pipeline.GetChildren(payloadBlockHandle.Get())) {
				declarationBlock += u8"layout(location="; ToString(declarationBlock, i); declarationBlock += u8") ";
				declarationBlock += targetSemantics == GAL::ShaderType::RAY_GEN ? u8"rayPayloadEXT " : u8"rayPayloadInEXT ";
				writeStructElement(declarationBlock, pipeline.GetMember(l)); declarationBlock += u8"\n";
				++i;
			}
		}();

		auto& main = pipeline.GetFunction(scopes, u8"main");

		[&] {
			auto shaderRecordBlockHandle = pipeline.TryGetElementHandle(scopes, u8"shaderRecordBlock");
			if (!shaderRecordBlockHandle) { return; }
			declarationBlock += u8"layout(shaderRecordEXT, scalar) buffer shader { ";
			for (const auto& l : pipeline.GetChildren(shaderRecordBlockHandle.Get())) { writeStructElement(declarationBlock, pipeline.GetMember(l)); }
			declarationBlock += u8" };\n";
		}();
	}

	switch (targetSemantics) {
	case GAL::ShaderType::VERTEX: {
		declarationBlock += u8"layout(location=0) out vertexData { ";

		[&] {
			auto vertexFragmentInterfaceBlockHandle = pipeline.TryGetElementHandle(scopes, u8"vertexSurfaceInterface");
			if (!vertexFragmentInterfaceBlockHandle) { addErrorCode(u8"Vertex-Surface interface block declaration was not found."); return; }
			for (auto& e : pipeline.GetChildren(vertexFragmentInterfaceBlockHandle.Get())) {
				if(pipeline.GetMember(e).Type == u8"uint32") {
					declarationBlock += u8"flat ";
				}
				writeStructElement(declarationBlock, pipeline.GetMember(e));
			}
		}();

		declarationBlock += u8" } vertexSurfaceInterface;\n";

		[&] {
			auto vertexBlockHandle = pipeline.TryGetElementHandle(scopes, u8"vertex");
			if (!vertexBlockHandle) { addErrorCode(u8"Vertex declaration was not found."); return; }
			for (uint8 i = 0; const auto & ve : pipeline.GetChildren(vertexBlockHandle.Get())) {
				declarationBlock += u8"layout(location="; ToString(declarationBlock, i); declarationBlock += u8") in ";
				writeStructElement(declarationBlock, pipeline.GetMember(ve)); declarationBlock += u8'\n';
				++i;
			}
		}();

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
		break;
	}
	case GAL::ShaderType::TESSELLATION_CONTROL: break;
	case GAL::ShaderType::TESSELLATION_EVALUATION: break;
	case GAL::ShaderType::GEOMETRY: break;
	case GAL::ShaderType::FRAGMENT: {
		declarationBlock += u8"layout(location=0) in vertexData { ";

		[&] {
			auto vertexFragmentInterfaceBlockHandle = pipeline.TryGetElementHandle(scopes, u8"vertexSurfaceInterface");
			if (!vertexFragmentInterfaceBlockHandle) { addErrorCode(u8"Vertex-Surface interface block declaration was not found."); return; }
			for (auto& e : pipeline.GetChildren(vertexFragmentInterfaceBlockHandle.Get())) {
				writeStructElement(declarationBlock, pipeline.GetMember(e));
			}
		}();

		declarationBlock += u8" } vertexIn;\n";

		[&] {
			auto fragmentOutputBlockHandle = pipeline.TryGetElementHandle(scopes, u8"fragmentOutputBlock");
			if (!fragmentOutputBlockHandle) { addErrorCode(u8"Fragment output block declaration was not found."); return; }
			for (uint32 i = 0; auto & e : pipeline.GetChildren(fragmentOutputBlockHandle.Get())) {
				declarationBlock += u8"layout(location="; ToString(declarationBlock, i); declarationBlock += u8") out ";
				writeStructElement(declarationBlock, pipeline.GetMember(e)); declarationBlock += u8'\n';
				++i;
			}
		}();

		break;
	}
	case GAL::ShaderType::COMPUTE: {
		auto xSize = pipeline.TryGetElementHandle(scopes, u8"group_size_x");
		auto ySize = pipeline.TryGetElementHandle(scopes, u8"group_size_y");
		auto zSize = pipeline.TryGetElementHandle(scopes, u8"group_size_z");
		declarationBlock += u8"layout(local_size_x="; declarationBlock += pipeline.GetMember(xSize.Get()).DefaultValue;
		declarationBlock += u8",local_size_y="; declarationBlock += pipeline.GetMember(ySize.Get()).DefaultValue;
		declarationBlock += u8",local_size_z="; declarationBlock += pipeline.GetMember(zSize.Get()).DefaultValue;
		declarationBlock += u8") in;\n";

		break;
	}
	case GAL::ShaderType::TASK:
		break;
	case GAL::ShaderType::RAY_GEN: {
		break;
	}
	case GAL::ShaderType::MISS: {
		break;
	}
	case GAL::ShaderType::CALLABLE:
		break;
	}

	writeFunction(functionBlock, pipeline.TryGetElementHandle(scopes, u8"main").Get(), pipeline.GetFunction(scopes, u8"main").Id, writeFunction); //add main

	GTSL::String<ALLOCATOR> fin(allocator);

	fin += headerBlock;
	fin += structBlock;
	fin += declarationBlock;
	fin += functionBlock;

	return GTSL::Result(GTSL::Pair(MoveRef(fin), MoveRef(errorString)), errorString.IsEmpty());
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
	//The set of Interface <id> must be equal to or a superset of the global OpVariable Result <id> referenced by the entry point�s static call tree,
	//within the interface�s storage classes. Before version 1.4, the interface�s storage classes are limited to the Input and Output storage classes.
	//Starting with version 1.4, the interface�s storage classes are all storage classes used in declaring all global variables
	//referenced by the entry point�s call tree.
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
//	switch (element.Type) {
//	break; case Shader::LanguageElement::ElementType::TYPE: handle = statement.Emplace(parentHandle, ShaderNode::ElementType::RVALUE, tokens[index]);
//	break; case Shader::LanguageElement::ElementType::MEMBER: handle = statement.Emplace(parentHandle, ShaderNode::ElementType::VARIABLE, tokens[index]);
//	break; case Shader::LanguageElement::ElementType::FUNCTION:
//		handle = statement.Emplace(parentHandle, isOperator ? ShaderNode::ElementType::OPERATOR : ShaderNode::ElementType::FUNCTION, tokens[index]);
//	break; default:;
//	}
//
//	if (element.Type == Shader::LanguageElement::ElementType::FUNCTION) {
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

//inline auto parseStatement(GTSL::JSONMember parent, GTSL::Tree<ShaderNode, BE::PAR>& tree, uint32 parentHandle) -> uint32 {
//	auto handle = tree.Emplace(parentHandle);
//	auto& node = tree[handle];
//
//	if (auto nameMember = parent[u8"name"]) { //var || var decl || func || operator
//		node.Name = GTSL::StringView(nameMember);
//
//		if (auto paramsMember = parent[u8"params"]) { //function, var decl
//			if (auto typeMember = parent[u8"type"]) { //name ^ params ^ type -> var decl
//				//node.ValueType = ShaderNode::ElementType::VAR_DEC;
//				//node.TypeName = GTSL::StringView(typeMember);
//			}
//			else { //name ^ params ^ ~type -> function
//				if (GTSL::IsSymbol(nameMember.GetStringView()[0])) {
//					node.ValueType = ShaderNode::ElementType::OPERATOR;
//				}
//				else if (nameMember.GetStringView() == u8"return") {
//					node.ValueType = ShaderNode::ElementType::RETURN;
//				}
//				else {
//					node.ValueType = ShaderNode::ElementType::FUNCTION;
//				}
//			}
//
//			for (auto e : parent[u8"params"]) {
//				parseStatement(e, tree, handle);
//			}
//		}
//		else { //name and no params -> var
//			node.ValueType = ShaderNode::ElementType::VARIABLE;
//		}
//	}
//	else if (auto outputMember = parent[u8"output"]) {
//		node.Name = outputMember;
//		//node.ValueType = ShaderNode::ElementType::SHADER_RESULT;
//		for (auto e : parent[u8"params"]) {
//			parseStatement(e, tree, handle);
//		}
//	}
//	else { //no name -> literal
//		if (auto valueMember = parent[u8"value"]) {
//			node.Name = valueMember;
//			node.ValueType = ShaderNode::ElementType::LITERAL;
//		}
//		else {
//			//node.TypeName = parent[u8"type"];
//			node.ValueType = ShaderNode::ElementType::RVALUE;
//			for (auto e : parent[u8"params"]) {
//				parseStatement(e, tree, handle);
//			}
//		}
//	}
//
//	return handle;
//}