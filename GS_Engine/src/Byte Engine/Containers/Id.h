#pragma once

#include "Core.h"
#include "FString.h"

class FString;

class Id64
{
public:
	using HashType = uint64;

	Id64() = default;

	Id64(const char* Text);

	explicit Id64(HashType id);
	
	explicit Id64(const FString& _Text);

	~Id64() = default;

	HashType GetID() { return hashValue; }
	[[nodiscard]] HashType GetID() const { return hashValue; }

	operator HashType() const { return hashValue; }

	static HashType HashString(const char* text);
	static HashType HashString(const FString& fstring);

	bool operator==(const Id64& other) { return hashValue == other.hashValue; }
	
private:
	HashType hashValue = 0;
	
	static HashType hashString(uint32 length, const char* text);
};

class Id32
{
	uint32 hash = 0;
	static uint32 hashString(uint32 stringLength, const char* str);
public:
	Id32(const char* text);
	Id32(uint32 length, const char* text);

	operator uint32() const { return hash; }
};

class Id16
{
	uint16 hash = 0;
	static uint16 hashString(uint32 stringLength, const char* str);
public:
	Id16(const char* text);

	operator uint16() const { return hash; }
};