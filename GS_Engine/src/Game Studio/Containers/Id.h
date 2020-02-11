#pragma once

#include "Core.h"

class FString;

class Id
{
public:
	using HashType = GS_HASH_TYPE;

	Id() = default;

	Id(const char* Text);

	explicit Id(HashType id);
	
	explicit Id(const FString& _Text);

	~Id() = default;

	INLINE HashType GetID() { return hashValue; }
	INLINE HashType GetID() const { return hashValue; }

	operator HashType() const { return hashValue; }

	static HashType HashString(const char* text);
	static HashType HashString(const FString& fstring);

	bool operator==(const Id& other) { return hashValue == other.hashValue; }
	
private:
	HashType hashValue = 0;

	static HashType hashString(uint32 length, const char* text);
};
