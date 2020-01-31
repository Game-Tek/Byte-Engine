#pragma once

#include "Core.h"

class FString;

class Id
{
public:
	using HashType = GS_HASH_TYPE;

	Id() = default;

	Id(const char* Text);

	Id(const FString& _Text);

	~Id() = default;

	INLINE HashType GetID() { return HashedString; }
	INLINE HashType GetID() const { return HashedString; }

	operator HashType() const { return HashedString; }

	static HashType HashString(const char* text);
	static HashType HashString(const FString& fstring);
private:
	HashType HashedString;
};
