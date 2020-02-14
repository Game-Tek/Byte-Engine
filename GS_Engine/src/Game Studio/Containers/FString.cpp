#include "FString.h"
#include <cstdio>
#include <cstdarg>

#include "Resources/Resource.h"
#include "Array.hpp"

OutStream& operator<<(OutStream& _Archive, FString& _String)
{
	_Archive << _String.Data;

	return _Archive;
}

InStream& operator>>(InStream& _Archive, FString& _String)
{
	_Archive >> _String.Data;

	return _Archive;
}

FString::FString() : Data(10)
{
}

FString::FString(const char* _In) : Data(StringLength(_In), CCAST(char*, _In))
{
}

FString::FString(char* const _In) : Data(StringLength(_In), CCAST(char*, _In))
{
}

FString::FString(length_type _Length) : Data(_Length)
{
}

FString::FString(const length_type _Length, const char* _In) : Data(_Length + 1, const_cast<char*>(_In))
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

bool FString::operator==(const FString& _Other) const
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
	Data.pop_back(); //Get rid of null terminator.
	Data.push_back(' '); //Push space.
	Data.push_back(StringLength(_In), const_cast<char*>(_In));
	return;
}

void FString::Append(const FString& _In)
{
	Data.pop_back(); //Get rid of null terminator.
	Data.push_back(' '); //Push space.
	Data.push_back(_In.Data); //Push new string.
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

void FString::Drop(int64 from)
{
	Data.resize(from + 1);
	Data[from + 1] = '\0';
}

void FString::ReplaceAll(char a, char with)
{
	for (uint32 i = 0; i < Data.getLength() - 1; ++i)
	{
		if (Data[i] == a)
		{
			Data[i] = with;
		}
	}
}

void FString::ReplaceAll(const char* a, const char* with)
{
	Array<uint32, 24, uint8> ocurrences; //cache ocurrences so as to not perform an array resize every time we find a match

	auto a_length = StringLength(a) - 1;
	auto with_length = StringLength(with) - 1;

	uint32 i = 0;
	
	while (true) //we don't know how long we will have to check for matches so keep looping until last if exits
	{
		ocurrences.resize(0); //every time we enter loop set occurrences to 0

		while(ocurrences.getLength() < ocurrences.getCapacity() && i < Data.getLength()) //while we don't exceed the occurrences array capacity and we are not at the end of the array(because we might hit the end in the first caching iteration)
		{
			if (Data [i] == a[0]) //if current char matches the a's first character enter whole word loop check
			{
				uint32 j = 1;
				
				for (; j < a_length; ++j) //if the a text is matched add occurrence else quickly escape loop and go to next whole string loop
				{
					if (Data[i + j] != a[j]) 
					{
						break;
					}
				}

				if (j == a_length - 1) //if loop found word insert occurrence and jump i by a's length
				{
					ocurrences.emplace_back(i + 1 - a_length);
					i += a_length;
				}
			}
			else //current char is not a match just check next in the following iteration
			{
				++i;
			}
		}

		const auto resize_size = ocurrences.getLength() * (with_length - a_length);

		if (resize_size > 0)
		{
			Data.resize(Data.getLength() + resize_size);
		}

		for (auto& e : ocurrences)
		{
			Data.insert(e, with_length - a_length);
			Data.overwrite(with_length, const_cast<string_type*>(with), e);
		}

		if (i == Data.getLength() - 1) //if current index is last index in whole string break out of the loop
		{
			break;
		}
	}
}

constexpr FString::length_type FString::StringLength(const char* In)
{
	length_type Length = 0;

	while (In[Length] != '\0')
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
	const auto Count = snprintf(Return.Data.getData(), Return.Data.getLength(), _Text, vaargs) + 1;
	//Take into account null terminator.
	if (Count > Return.Data.getLength())
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
