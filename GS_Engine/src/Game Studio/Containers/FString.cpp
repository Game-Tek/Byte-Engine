#include "FString.h"
#include <cstdio>
#include <cstdarg>

FString::FString() : Data(10)
{
}

FString::FString(const char* _In) : Data(StringLength(_In), CCAST(char*, _In))
{
}

FString::FString(char* const _In) : Data(StringLength(_In), CCAST(char*, _In))
{
}

FString::FString(size_t _Length) : Data(_Length)
{
}

FString::FString(const size_t _Length, const char* _In) : Data(_Length + 1, const_cast<char*>(_In))
{
	Data.push_back('\0');
}

FString& FString::operator=(const char* _In)
{
	Data.recreate(StringLength(_In), const_cast<char*>(_In));
	return *this;
}

FString FString::operator+(const char* _In) const
{
	FString result;
	result.Data.push_back(Data.getLength() - 1, Data.getData());
	result.Data.push_back(StringLength(_In), _In);
	return result;
}

FString& FString::operator+=(const char* _In)
{
	Data.pop_back();
	Data.push_back(StringLength(_In), _In);
	return *this;
}

FString FString::operator+(const FString& _Other) const
{
	FString result;
	result.Data.push_back(Data.getLength() - 1, Data.getData());
	result.Data.push_back(_Other.Data.getLength(), _Other.Data.getData());
	return result;
}

bool FString::operator==(const FString & _Other) const
{
	//Discard if Length of strings is not equal, first because it helps us discard before even starting, second because we can't compare strings of different sizes.
	if (Data.getLength() != _Other.Data.getLength()) return false;

	for (size_t i = 0; i < Data.getLength(); i++)
	{
		if (Data[i] != _Other.Data[i])
		{
			return false;
		}
	}

	return true;
}

bool FString::NonSensitiveComp(const FString& _Other) const
{
	//Discard if Length of strings is not equal, first because it helps us discard before even starting, second because we can't compare strings of different sizes.
	if (Data.getLength() != _Other.Data.getLength()) return false;

	for (size_t i = 0; i < Data.getLength(); i++)
	{
		if (Data[i] != (ToLowerCase(_Other.Data[i]) || ToUpperCase(_Other.Data[i])))
		{
			return false;
		}
	}

	return true;
}

void FString::Append(const char* _In)
{
	Data.pop_back();											//Get rid of null terminator.
	Data.push_back(' ');									//Push space.
	Data.push_back(StringLength(_In), const_cast<char*>(_In));
	return;
}

void FString::Append(const FString& _In)
{
	Data.pop_back();			//Get rid of null terminator.
	Data.push_back(' ');	//Push space.
	Data.push_back(_In.Data);	//Push new string.
	return;
}

void FString::Insert(const char* _In, const size_t _Index)
{
	Data.push(_Index, const_cast<char*>(_In), StringLength(_In));
	return;
}

int64 FString::FindLast(char _Char) const
{
	for (int32 i = Data.getLength(); i > 0; --i)
	{
		if (Data[i] == _Char) return i;
	}

	return -1;
}

constexpr size_t FString::StringLength(const char * In)
{
	size_t Length = 0;

	while(In[Length] != '\0')
	{
		Length++;
	}

	//We return Length + 1 to take into account for the null terminator character.
	return Length + 1;
}

#define FSTRING_MAKESTRING_DEFAULT_SIZE 256

FString FString::MakeString(const char* _Text, ...)
{
	FString Return(FSTRING_MAKESTRING_DEFAULT_SIZE);

	va_list vaargs;
	va_start(vaargs, _Text);
	const auto Count = snprintf(Return.Data.getData(), Return.Data.getLength(), _Text, vaargs) + 1; //Take into account null terminator.
	if(Count > Return.Data.getLength())
    {
        Return.Data.resize(Count);

        snprintf(Return.Data.getData(), Return.Data.getLength(), _Text, vaargs);
    }
	va_end(vaargs);

	return Return;
}

char FString::ToLowerCase(char _Char)
{
	if ('A' <= _Char && _Char <= 'Z') return _Char += ('a' - 'A');
	return _Char;
}

char FString::ToUpperCase(char _Char)
{
	if ('a' <= _Char && _Char <= 'z') return _Char += ('a' - 'A');
	return _Char;
}
