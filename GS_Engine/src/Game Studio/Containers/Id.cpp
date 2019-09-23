#include "Id.h"

#include "FString.h"

Id::Id(const char * Text) : HashedString(HashString(Text))
{
}

Id::Id(const FString& _Text) : HashedString(HashString(_Text))
{
}

uint32 Id::HashString(const char * Text)
{
	const uint32 Length = FString::StringLength(Text);

	uint32 Hash = 0;

	for (uint32 i = 0; i < Length; i++)
	{
		Hash += Text[i] * i * 33;
	}

	Hash = Hash * Length * 5;

	return Hash;
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
