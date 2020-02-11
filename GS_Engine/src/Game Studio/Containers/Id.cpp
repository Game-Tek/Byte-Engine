#include "Id.h"

#include "FString.h"
#include "Ranger.h"

Id::Id(const FString& _Text) : hashValue(HashString(_Text))
{
}


Id::Id(const char* Text): hashValue(HashString(Text))
{
}

Id::Id(const HashType id) : hashValue(id)
{
}

Id::HashType Id::HashString(const char* text) { return hashString(FString::StringLength(text) - 1, text); };

Id::HashType Id::HashString(const FString& fstring) { return hashString(fstring.GetLength(), fstring.c_str()); }

Id::HashType Id::hashString(const uint32 length, const char* text)
{
	HashType primaryHash(525201411107845655ull);
	
	for (auto& c : Ranger(length, text))
	{
		primaryHash ^= c;
		primaryHash *= 0x5bd1e9955bd1e995;
		primaryHash ^= primaryHash >> 47;
	}

	HashType secondaryHash(0xAAAAAAAAAAAAAAAA);

	for (auto& c : Ranger(length, text))
	{
		secondaryHash ^= c;
		secondaryHash *= 0x80638e;
		secondaryHash ^= primaryHash >> 35;
	}

	primaryHash ^= secondaryHash + 0x9e3779b9 + (primaryHash << 6) + (primaryHash >> 2);
	
	return primaryHash;
}
