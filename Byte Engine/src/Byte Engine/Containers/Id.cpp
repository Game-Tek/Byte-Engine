#include "Id.h"

#include "FString.h"
#include "Ranger.h"

Id64::Id64(const FString& _Text) : hashValue(HashString(_Text))
{
}


Id64::Id64(const char* Text): hashValue(HashString(Text))
{
}

Id64::Id64(const HashType id) : hashValue(id)
{
}

Id64::HashType Id64::HashString(const char* text) { return hashString(FString::StringLength(text) - 1, text); };

Id64::HashType Id64::HashString(const FString& fstring) { return hashString(fstring.GetLength(), fstring.c_str()); }

Id64::HashType Id64::hashString(const uint32 length, const char* text)
{
	HashType primaryHash(525201411107845655ull);
	HashType secondaryHash(0xAAAAAAAAAAAAAAAA);
	
	for (auto& c : Ranger(length, text))
	{
		primaryHash ^= c;
		secondaryHash ^= c;
		primaryHash *= 0x5bd1e9955bd1e995;
		secondaryHash *= 0x80638e;
		primaryHash ^= primaryHash >> 47;
		secondaryHash ^= secondaryHash >> 35;
	}

	//primaryHash ^= secondaryHash + 0x9e3779b9 + (primaryHash << 6) + (primaryHash >> 2);
	
	return ((primaryHash & 0xFFFFFFFF00000000) ^ (secondaryHash & 0x00000000FFFFFFFF));
}

uint32 Id32::hashString(const uint32 stringLength, const char* str)
{
	uint32 primaryHash(525410765);
	uint32 secondaryHash(0xAAAAAAAA);

	for (auto& c : Ranger(stringLength, str))
	{
		primaryHash ^= c;
		secondaryHash ^= c;
		primaryHash *= 0x5bd1e995;
		secondaryHash *= 0x80638e;
		primaryHash ^= primaryHash >> 17;
		secondaryHash ^= secondaryHash >> 29;
	}

	return ((primaryHash & 0xFFFF0000) ^ (secondaryHash & 0x0000FFFF));
}

Id32::Id32(const char* text) : hash(hashString(FString::StringLength(text) - 1, text))
{
}

Id32::Id32(uint32 length, const char* text) : hash(hashString(length, text))
{
}

uint16 Id16::hashString(const uint32 stringLength, const char* str)
{
	uint16 primaryHash(52541);
	uint16 secondaryHash(0xAAAA);

	for (auto& c : Ranger(stringLength, str))
	{
		primaryHash ^= c;
		secondaryHash ^= c;
		primaryHash *= 0x5bd1e95;
		secondaryHash *= 0x8063e;
		primaryHash ^= primaryHash >> 9;
		secondaryHash ^= secondaryHash >> 11;
	}

	return ((primaryHash & 0xFF00) ^ (secondaryHash & 0x00FF));
}

Id16::Id16(const char* text) : hash(hashString(FString::StringLength(text) - 1, text))
{
}
