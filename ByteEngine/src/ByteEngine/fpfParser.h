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
};

template<class ALLOCATOR>
struct FileDescription
{
	GTSL::Tree<ClassDesciptor, ALLOCATOR> Classes;

	uint32 DataStart = 0xFFFFFFFF;
};

template<class ALLOCATOR>
inline bool BuildFileDescription(const GTSL::Range<const utf8*> text, const ALLOCATOR& allocator, FileDescription<ALLOCATOR>& fileDescription)
{
	uint32 c = 0;
	using Token = GTSL::StaticString<64>;

	fileDescription.Classes.Initialize(allocator);
	
	GTSL::FlatHashMap<GTSL::StaticString<64>, ALLOCATOR> registeredTypes(16, allocator);
	registeredTypes.Emplace(Id("uint32")(), "uint32"); registeredTypes.Emplace(Id("float32")(), "float32");
	registeredTypes.Emplace(Id("string")(), "string");

	
	GTSL::Array<GTSL::StaticString<64>, 128> tokens;

	{
		utf8 character = text[c];
		
		for (; c < text.ElementCount(); ++c)
		{
			auto advance = [&]() { auto oldChar = character; character = text[++c]; return oldChar; };

			auto accumUntilSkip = [&]()
			{
				GTSL::StaticString<512> string;

				while (GTSL::IsWhitespace(character)) { advance(); } //skip leading whitespace
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
				
				classPointer->Data.Members.EmplaceBack(Id(advance()));
			}

			return true;
		};
		
		for (; tokenIndex < tokens.GetLength(); ++tokenIndex) {
			if (advance() == "class") { if (!processClass()) { return false; }; }
		}
	}
	
	return true;
}

template<class ALLOCATOR>
inline bool ConsumeData(const GTSL::Range<const utf8*> text, const ALLOCATOR& allocator, const FileDescription<ALLOCATOR>& fileDescription)
{
	if (fileDescription.DataStart == 0xFFFFFFFF) { return false; }
}