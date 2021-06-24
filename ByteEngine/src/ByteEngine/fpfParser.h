#pragma once
#include <GTSL/Range.h>
#include <GTSL/StaticString.hpp>
#include <GTSL/Tree.hpp>

#include "Core.h"

struct ClassDesciptor
{
	struct ClassMember
	{
		GTSL::StaticString<64> Type, Name;
	};
	
	GTSL::Array<ClassMember, 16> Members;
	GTSL::StaticMap<Id, uint32, 16> MembersByName;
};

template<class ALLOCATOR>
struct FileDescription
{
	GTSL::Tree<ClassDesciptor, ALLOCATOR> Classes;
	GTSL::HashMap<Id, ClassDesciptor*, ALLOCATOR> ClassesByName;
	
	uint32 DataStart = 0xFFFFFFFF;
};

template<class ALLOCATOR>
bool BuildFileDescription(const GTSL::Range<const utf8*> text, const ALLOCATOR& allocator, FileDescription<ALLOCATOR>& fileDescription)
{
	uint32 c = 0;
	using Token = GTSL::StaticString<64>;

	fileDescription.Classes.Initialize(allocator); fileDescription.ClassesByName.Initialize(16, allocator);
	
	GTSL::HashMap<Id, GTSL::StaticString<64>, ALLOCATOR> registeredTypes(16, allocator);
	registeredTypes.Emplace(Id("uint32"), "uint32"); registeredTypes.Emplace(Id("float32"), "float32");
	registeredTypes.Emplace(Id("string"), "string");

	
	GTSL::Array<GTSL::StaticString<64>, 128> tokens;

	{
		utf8 character = text[c];
		
		for (; c < text.ElementCount();)
		{
			auto advance = [&]() { auto oldChar = character; character = text[++c]; return oldChar; };

			auto accumUntilSkip = [&]()
			{
				GTSL::StaticString<512> string;

				while (GTSL::IsWhitespace(character) || GTSL::IsSpecialCharacter(character)) { advance(); } //skip leading whitespace
				while (!GTSL::IsWhitespace(character) && !GTSL::IsSpecialCharacter(character)) { //collect until break
					string += advance();
				}

				return string;
			};

			auto foundIdentifier = [&]()
			{
				auto identifier = accumUntilSkip();
				tokens.EmplaceBack(identifier);
			};

			foundIdentifier();

			if (tokens.GetLength() > 1) {
				if (tokens[tokens.GetLength() - 2] == "}" && tokens.back() == "{") {
					tokens.PopBack();

					while (true) {
						if (text[c] == '{') {
							fileDescription.DataStart = c; break;
						}

						--c;
					}
					
					break;
				}
			}
			//stop tokenizing when we found last class declaration, what follows is data and should only be parsed after this
		}
	}

	{
		uint32 tokenIndex = 0; Token token;

		auto advance = [&]() { auto oldToken = tokens[tokenIndex]; token = tokens[++tokenIndex]; return oldToken; };
		
		auto processClass = [&]() -> bool {
			
			auto classNameToken = advance();
			auto hashedName = Id(classNameToken);
			auto result = registeredTypes.TryEmplace(hashedName, classNameToken);
			if (!result.State()) { return false; } //symbol with that name already existed
			
			
			auto* classPointer = fileDescription.Classes.AddChild(nullptr);
			if (advance() != "{") { return false; }
			fileDescription.ClassesByName.Emplace(hashedName, &classPointer->Data);

			while (token != "}") {
				{
					auto memberType = advance();

					if (memberType.Find("[]")) { //register array class
						auto hashedName = Id(memberType);
						auto placeAttempt = registeredTypes.TryEmplace(hashedName, memberType);
						
						if (placeAttempt.State()) {
							auto* node = fileDescription.Classes.AddChild(nullptr);
							fileDescription.ClassesByName.Emplace(hashedName, &node->Data);
							auto nonArrayType = memberType;
							nonArrayType.Drop(nonArrayType.FindLast('[').Get());
							node->Data.Members.EmplaceBack(nonArrayType, "arrMem");
						}
					}
					
					{
						//if (!registeredTypes.Find(Id(memberType))) { return false; }

						auto memberName = advance();
						auto memberIndex = classPointer->Data.Members.EmplaceBack(memberType, memberName);
						classPointer->Data.MembersByName.Emplace(Id(memberName), memberIndex);
					}
				}
			}

			return true;
		};
		
		for (; tokenIndex < tokens.GetLength(); ++tokenIndex) {
			if (advance() == "class") { if (!processClass()) { return false; }; }
		}
	}
	
	return true;
}

template<typename ALLOCATOR>
struct ParseState
{
	struct StackState
	{
		GTSL::StaticString<32> Type, Name; uint32 C = 0, Index = 0;
	};
	GTSL::Vector<StackState, ALLOCATOR> Stack;
	GTSL::Range<const utf8*> Text;
	uint32 C = 0; utf8 Character = 0;
	GTSL::StaticString<32> LasToken;
	//uint32 Scope = 0;

	template<class ALLOC>
	auto advance(FileDescription<ALLOC>& fileDescription) {
		if (Character == '{')
		{
			if (Stack.GetLength())
			{
				auto& stackState = Stack.back();
				auto& cl = fileDescription.ClassesByName.At(Id(stackState.Type));
				auto& member = cl->Members[stackState.BlockIndex % cl->Members.GetLength()];

				{
					StackState stackState;
					stackState.Type = member.Type;
					stackState.InputSource = member.InputSource;
					stackState.C = C; //todo
					Stack.EmplaceBack(stackState);
				}
			}
		} else if (Character == '}') {
			Stack.PopBack();
		} else if (Character == ',') {
			++Stack.back().BlockIndex;
		}
		
		auto oldChar = Character; Character = Text[++C];
		
		return oldChar;
	}

	template<class ALLOC>
	auto accumUntilSkip(FileDescription<ALLOC>& fileDescription)
	{
		GTSL::StaticString<512> string;

		while (GTSL::IsWhitespace(Character) || GTSL::IsSpecialCharacter(Character)) { advance(fileDescription); } //skip leading whitespace
		while (!GTSL::IsWhitespace(Character) && !GTSL::IsSpecialCharacter(Character)) { //collect until break
			string += advance(fileDescription);
		}

		return string;
	};

	template<class ALLOC>
	auto accumUntilSkipWithSymbols(FileDescription<ALLOC>& fileDescription)
	{
		GTSL::StaticString<512> string;

		while (GTSL::IsWhitespace(Character) || GTSL::IsSpecialCharacter(Character) || GTSL::IsSymbol(Character)) { advance(fileDescription); } //skip leading whitespace
		while (!GTSL::IsWhitespace(Character) && !GTSL::IsSpecialCharacter(Character) && !GTSL::IsSymbol(Character)) { //collect until break
			string += advance(fileDescription);
		}

		return string;
	};
};

template<class ALLOC1, class ALLOC2>
bool StartParse(FileDescription<ALLOC1>& fileDescription, ParseState<ALLOC2>& parseState, const GTSL::Range<const utf8*> text, const ALLOC2& allocator)
{
	if (fileDescription.DataStart == 0xFFFFFFFF) { return false; }
	parseState.Stack.Initialize(16, allocator);
	parseState.C = fileDescription.DataStart;
	parseState.Text = text;
	parseState.Character = text[parseState.C];
}

template<class ALLOC1, class ALLOC2>
bool GoToArray(FileDescription<ALLOC1>& fileDescription, ParseState<ALLOC2>& parseState, GTSL::StaticString<64> variableName, uint32 index = 0)
{
	if(parseState.Stack.GetLength() == 0)
	{
		GTSL::StaticString<256> strings[2];
		
		while (true) {
			strings[1] = parseState.accumUntilSkip(fileDescription);

			if (strings[1] == variableName) {
				auto* node = fileDescription.Classes.AddChild(nullptr);
				fileDescription.ClassesByName.Emplace(Id(strings[0]), &node->Data);
				auto nonArrayType = strings[0];
				nonArrayType.Drop(nonArrayType.FindLast('[').Get());
				node->Data.Members.EmplaceBack(nonArrayType, "arrMem");

				typename ParseState<ALLOC2>::StackState stackState;
				stackState.Type = strings[0];
				stackState.InputSource = strings[1];
				stackState.C = parseState.C; //todo
				parseState.Stack.EmplaceBack(stackState);

				parseState.C += 2;
				parseState.Character = parseState.Text[parseState.C];
				
				break;
			}

			strings[0] = strings[1];
		}
	}
	else
	{
		auto& e = fileDescription.ClassesByName.At(Id(parseState.Stack.back().Type));
		uint32 memberIndex = e->MembersByName.At(Id(variableName));

		while (true) { //fix
			if (parseState.Stack.back().BlockIndex == memberIndex) { GoToIndex(fileDescription, parseState); break; }
			parseState.advance(fileDescription);
		}
	}
	
	return true;
}

template<class ALLOC1, class ALLOC2>
bool GoToIndex(FileDescription<ALLOC1>& fileDescription, ParseState<ALLOC2>& parseState, uint32 index = 0)
{
	auto& stackState = parseState.Stack.back();

	auto scope = parseState.Stack.GetLength();
	
	while (true) { //fix
		parseState.advance(fileDescription);
		if (parseState.Stack.GetLength() > scope) { return true; }
		if (parseState.Stack.GetLength() < scope)
		{
			auto scope = parseState.Stack.GetLength();
			
			while (true) {
				parseState.advance(fileDescription);
				if (parseState.Stack.GetLength() > scope) { return true; }
				if (parseState.Stack.GetLength() < scope) { return false; }
			}
		}
	}
}

//template<class ALLOCATOR, typename T>
//bool GetVariable(const FileDescription<ALLOCATOR>& fileDescription, ParseState<ALLOCATOR>& parseState, GTSL::StaticString<32> objectName, T& obj);

template<class ALLOCATOR>
bool GetVariable(FileDescription<ALLOCATOR>& fileDescription, ParseState<ALLOCATOR>& parseState, GTSL::StaticString<32> objectName, uint32& obj)
{	
	auto& e = fileDescription.ClassesByName.At(Id(parseState.Stack.back().Type));
	uint32 memberIndex = e->MembersByName.At(Id(objectName));

	while (true) { //fix
		if (e->Members[parseState.Stack.back().BlockIndex % e->Members.GetLength()].InputSource == objectName) { break; }
		parseState.advance(fileDescription);
	}
	
	auto res = parseState.accumUntilSkipWithSymbols(fileDescription);
	auto number = GTSL::ToNumber<uint32>(res);
	if (!number.State()) { return false; }
	obj = number.Get();
	
	return true;
}