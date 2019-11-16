#include "Id.h"

#include "FString.h"

Id::Id(const FString& _Text) : HashedString(HashString(_Text))
{
}

Id::HashType Id::HashString(const char* Text)
{
	const auto Length = FString::StringLength(Text) - 1;

	HashType Hash = 0;

	for (size_t i = 0; i < Length; i++)
	{
		Hash += Text[i] * (i / 5) * 33;
	}

	Hash = Hash * Length * 5;

	return Hash;
}

Id::Id(const char* Text): HashedString(HashString(Text))
{
}

Id::HashType Id::HashString(const FString& _Text)
{
	uint32 Hash = 0;

	for (uint32 i = 0; i < _Text.GetLength(); i++)
	{
		Hash += _Text[i] * i * 33;
	}

	Hash = Hash * _Text.GetLength() * 5;

	return Hash;
}
