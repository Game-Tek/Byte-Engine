#pragma once
#include <GTSL/Range.h>
#include <GTSL/StaticString.hpp>


#include "Core.h"

struct ClassDesciptor
{
	struct ClassMember
	{
		Id Name; uint32 Size, Offset;
	};
	
	GTSL::Array<ClassMember, 16> Members;
	GTSL::StaticMap<uint32, 16> MembersByName;
};

template<class ALLOCATOR>
struct FileDescription
{
	GTSL::Tree<ClassDesciptor, ALLOCATOR> Classes;
	GTSL::FlatHashMap<Id, ClassDesciptor*, ALLOCATOR> ClassesByName;
	
	uint32 DataStart = 0xFFFFFFFF;
};

template<class ALLOCATOR>
inline bool BuildFileDescription(const GTSL::Range<const utf8*> text, const ALLOCATOR& allocator, FileDescription<ALLOCATOR>& fileDescription)
{
	uint32 c = 0;
	using Token = GTSL::StaticString<64>;

	fileDescription.Classes.Initialize(allocator); fileDescription.ClassesByName.Initialize(16, allocator);
	
	GTSL::FlatHashMap<Id, GTSL::StaticString<64>, ALLOCATOR> registeredTypes(16, allocator);
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
				tokens.EmplaceBack(Id(identifier));
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
			{
				auto classNameToken = advance();
				auto result = registeredTypes.TryEmplace(Id(classNameToken)(), classNameToken);
				if (!result.State()) { return false; } //symbol with that name already existed
			}
			
			auto* classPointer = fileDescription.Classes.AddChild(nullptr);
			if (advance() != "{") { return false; }

			while (token != "}") {
				{
					auto memberType = advance();

					if (memberType.Find("[]")) {
						/*make array*/
						memberType.Drop(memberType.GetLength() - 3);
					}
					
					if (!registeredTypes.Find(Id(memberType)())) { return false; }
					
				}

				{
					auto memberName = advance(); auto hashedMeberName = Id(memberName);
					auto memberIndex = classPointer->Data.Members.EmplaceBack(hashedMeberName);
					classPointer->Data.MembersByName.Emplace(hashedMeberName(), memberIndex);
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
		GTSL::StaticString<32> Type; uint32 C = 0;
	};
	GTSL::Vector<StackState, ALLOCATOR> Stack;
	GTSL::Range<const utf8*> Text;
	uint32 C = 0; utf8 Character = 0;
	GTSL::StaticString<32> LasToken;
	uint32 Scope = 0;
	
	auto advance() { auto oldChar = Character; Character = Text[++C]; return oldChar; };
	
	auto accumUntilSkip()
	{
		GTSL::StaticString<512> string;

		while (GTSL::IsWhitespace(Character) || GTSL::IsSpecialCharacter(Character)) { advance(); } //skip leading whitespace
		while (!GTSL::IsWhitespace(Character) && !GTSL::IsSpecialCharacter(Character)) { //collect until break
			string += advance();
		}

		return string;
	};

	auto accumUntilSkipWithSymbols()
	{
		GTSL::StaticString<512> string;

		while (GTSL::IsWhitespace(Character) || GTSL::IsSpecialCharacter(Character) || GTSL::IsSymbol(Character)) { advance(); } //skip leading whitespace
		while (!GTSL::IsWhitespace(Character) && !GTSL::IsSpecialCharacter(Character) && !GTSL::IsSymbol(Character)) { //collect until break
			string += advance();
		}

		return string;
	};
};

template<class ALLOC1, class ALLOC2>
inline bool StartParse(FileDescription<ALLOC1>& fileDescription, ParseState<ALLOC2>& parseState, const GTSL::Range<const utf8*> text, const ALLOC2& allocator)
{
	if (fileDescription.DataStart == 0xFFFFFFFF) { return false; }
	parseState.Stack.Initialize(16, allocator);
	parseState.C = fileDescription.DataStart;
	parseState.Text = text;
	parseState.Character = text[parseState.C];
}

template<class ALLOC1, class ALLOC2>
inline bool GoToVariable(FileDescription<ALLOC1>& fileDescription, ParseState<ALLOC2>& parseState, GTSL::StaticString<64> variableName, uint32 index = 0)
{
	//{
	//	auto find = parseState.Stack.Find(variableName);
	//	if(find.State()) {
	//		parseState.Stack.Pop(find.Get(), parseState.Stack.GetLength() - find.Get());
	//
	//		while (true)
	//		{
	//			auto parseResult = parseState.accumUntilSkip();
	//
	//			
	//		}
	//	}
	//}

	if(parseState.Scope == 0)
	{
		while (true) {
			auto parseResult = parseState.accumUntilSkipWithSymbols();

			if(fileDescription.ClassesByName.Find(Id(parseResult)())) {
				typename ParseState<ALLOC2>::StackState stackState;
				stackState.Type = parseResult;
				stackState.C = parseState.C; //todo
				parseState.Stack.EmplaceBack(stackState);
			}

			if (parseResult == "{") { ++parseState.Scope; continue; }
			if (parseResult == "}") { --parseState.Scope; continue; }
		}
	}
	else
	{
		
	}
	
	return true;
}

template<class ALLOCATOR, typename T>
bool GetVariable(const FileDescription<ALLOCATOR>& fileDescription, ParseState<ALLOCATOR>& parseState, GTSL::StaticString<32> objectName, T& obj);

template<class ALLOCATOR, uint32>
bool GetVariable(const FileDescription<ALLOCATOR>& fileDescription, ParseState<ALLOCATOR>& parseState, GTSL::StaticString<32> objectName, uint32& obj)
{	
	auto& e = fileDescription.ClassesByName.At(Id(parseState.Stack.back().Type)());
	uint32 memberIndex = e->MembersByName.Find(Id(objectName)());

	uint32 index = 0;
	
	while(index != memberIndex) {
		if (parseState.Text[parseState.C++] == ',') { ++index; }
	}

	auto res = parseState.accumUntilSkipWithSymbols();
	auto number = GTSL::ToNumber<uint32>(res);
	if (!number.State()) { return false; }
	obj = number.Get();
	
	return true;
}