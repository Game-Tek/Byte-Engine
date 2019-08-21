#include "FString.h"

FString::FString() : Data(10)
{
}

FString::FString(const char * In) : Data(const_cast<char *>(In), StringLength(In))
{
}

FString::FString(const char * In, const size_t Length) : Data(const_cast<char *>(In), Length + 1)
{
	Data.push_back('\0');
}

FString & FString::operator=(const char * In)
{
	Data.recreate(const_cast<char *>(In), StringLength(In));

	return *this;
}

FString & FString::operator+(const char * Other)
{
	Data.push_back(const_cast<char *>(Other), StringLength(Other));

	return *this;
}

FString & FString::operator+(const FString & Other)
{
	Data.push_back(Other.Data);

	return *this;
}

bool FString::operator==(const FString & Other) const
{
	for (size_t i = 0; i < (Data.length() < Other.Data.length() ? Data.length() : Other.Data.length()); i++)
	{
		if(Data[i] != Other.Data[i])
		{
			return false;
		}
	}

	return true;
}

char * FString::c_str()
{
	return Data.data();
}

const char * FString::c_str() const
{
	return Data.data();
}

void FString::Append(const char * In)
{
	Data.push_back(' ');

	Data.push_back(const_cast<char *>(In), StringLength(In));

	return;
}

void FString::Append(const FString & In)
{
	Data.push_back(' ');

	Data.push_back(In.Data);

	return;
}

void FString::Insert(const char * In, const size_t Index)
{
	Data.insert(Index, const_cast<char *>(In), StringLength(In));

	return;
}

int64 FString::FindLast(char _Char) const
{
	for (int32 i = Data.length(); i > 0; --i)
	{
		if (Data[i] == _Char) return i;
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
