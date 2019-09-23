#pragma once

#include "Core.h"

class FString;

class GS_API Id
{
public:
	using HashType = uint32;

	Id() = default;
	explicit Id(const char * Text);
	explicit Id(const FString& _Text);
	~Id() = default;

	INLINE HashType GetID() { return HashedString; }
	INLINE HashType GetID() const { return HashedString; }
private:
	uint32 HashedString;

	static HashType HashString(const char* Text);
	static HashType HashString(const FString& _Text);
};

