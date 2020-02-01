#include "Id.h"

#include "FString.h"

Id::Id(const FString& _Text) : HashedString(HashString(_Text))
{
}

Id::HashType Id::HashString(const char* text)
{
	const auto Length = FString::StringLength(text) - 1;

	HashType h(525201411107845655ull);
	for (; *text; ++text)
	{
		h ^= *text;
		h *= 0x5bd1e9955bd1e995;
		h ^= h >> 47;
	}
	return h;
}

Id::Id(const char* Text): HashedString(HashString(Text))
{
}

Id::Id(const HashType id) : HashedString(id)
{
}

Id::HashType Id::HashString(const FString& fstring)
{
	HashType h(525201411107845655ull);
	for (auto c : fstring)
	{
		h ^= c;
		h *= 0x5bd1e9955bd1e995;
		h ^= h >> 47;
	}
	return h;
}
