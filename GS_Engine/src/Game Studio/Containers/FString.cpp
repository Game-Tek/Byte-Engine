#include "FString.h"

FString::FString() : Array(10)
{
}

FString::FString(const char * In) : Array(const_cast<char *>(In), StringLength(In))
{
}

FString::FString(const char * In, const size_t Length) : Array(const_cast<char *>(In), Length + 1)
{
	Array.push_back('\0');
}

FString & FString::operator=(const char * In)
{
	Array.recreate(const_cast<char *>(In), StringLength(In));

	return *this;
}

FString & FString::operator+(const char * Other)
{
	Array.push_back(const_cast<char *>(Other), StringLength(Other));

	return *this;
}

FString & FString::operator+(const FString & Other)
{
	Array.push_back(Other.Array);

	return *this;
}

bool FString::operator==(const FString & Other) const
{
	for (size_t i = 0; i < (Array.length() < Other.Array.length() ? Array.length() : Other.Array.length()); i++)
	{
		if(Array[i] != Other.Array[i])
		{
			return false;
		}
	}

	return true;
}

char * FString::c_str()
{
	return Array.data();
}

const char * FString::c_str() const
{
	return Array.data();
}

void FString::Append(const char * In)
{
	Array.push_back(' ');

	Array.push_back(const_cast<char *>(In), StringLength(In));

	return;
}

void FString::Append(const FString & In)
{
	Array.push_back(' ');

	Array.push_back(In.Array);

	return;
}

void FString::Insert(const char * In, const size_t Index)
{
	Array.insert(Index, const_cast<char *>(In), StringLength(In));

	return;
}

int64 FString::FindLast(char _Char) const
{
	for (int32 i = Array.length(); i > 0; --i)
	{
		if (Array[i] == _Char) return i;
	}

	return -1;
}

size_t FString::StringLength(const char * In)
{
	size_t Length = 0;

	while(In[Length] != '\0')
	{
		Length++;
	}

	//We return Length + 1 to take into account for the null terminator character.
	return Length + 1;
}
