#include "FString.h"
#include <cstdio>
#include <cstdarg>

#include "Resources/Resource.h"
#include "Array.hpp"
#include <string>

OutStream& operator<<(OutStream& _Archive, FString& _String)
{
	_Archive << _String.data;

	return _Archive;
}

InStream& operator>>(InStream& _Archive, FString& _String)
{
	_Archive >> _String.data;

	return _Archive;
}

FString::FString() : data(10)
{
}

FString::FString(char* const cstring) : data(StringLength(cstring), CCAST(char*, cstring))
{
}

FString& FString::operator=(const char* _In)
{
	data.recreate(StringLength(_In), const_cast<char*>(_In));
	return *this;
}

FString FString::operator+(const char* cstring) const
{
	FString result;
	result.data.push_back(data.getLength() - 1, data.getData());
	result.data.push_back(StringLength(cstring), cstring);
	return result;
}

FString& FString::operator+=(const char* cstring)
{
	data.pop_back();
	data.push_back(StringLength(cstring), cstring);
	return *this;
}

FString FString::operator+(const FString& _Other) const
{
	FString result;
	result.data.push_back(data.getLength() - 1, data.getData());
	result.data.push_back(_Other.data.getLength(), _Other.data.getData());
	return result;
}

bool FString::operator==(const FString& _Other) const
{
	//Discard if Length of strings is not equal, first because it helps us discard before even starting, second because we can't compare strings of different sizes.
	if (data.getLength() != _Other.data.getLength()) return false;

	for (size_t i = 0; i < data.getLength(); i++)
	{
		if (data[i] != _Other.data[i])
		{
			return false;
		}
	}

	return true;
}

bool FString::NonSensitiveComp(const FString& _Other) const
{
	//Discard if Length of strings is not equal, first because it helps us discard before even starting, second because we can't compare strings of different sizes.
	if (data.getLength() != _Other.data.getLength()) return false;

	for (size_t i = 0; i < data.getLength(); i++)
	{
		if (data[i] != (ToLowerCase(_Other.data[i]) || ToUpperCase(_Other.data[i])))
		{
			return false;
		}
	}

	return true;
}

void FString::Append(const char* _In)
{
	data.pop_back(); //Get rid of null terminator.
	data.push_back(' '); //Push space.
	data.push_back(StringLength(_In), const_cast<char*>(_In));
	return;
}

void FString::Append(const FString& _In)
{
	data.pop_back(); //Get rid of null terminator.
	data.push_back(' '); //Push space.
	data.push_back(_In.data); //Push new string.
	return;
}

#include <stdlib.h>

void FString::Append(const int_64 number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 50);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%lld", number) + 1);
}

void FString::Append(float number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 50);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%f", number) + 1);
}

void FString::Insert(const char* _In, const size_t _Index)
{
	data.push(_Index, const_cast<char*>(_In), StringLength(_In));
	return;
}

FString::length_type FString::FindLast(char _Char) const
{
	for (int32 i = data.getLength(); i > 0; --i)
	{
		if (data[i] == _Char) return i;
	}

	return npos();
}

FString::length_type FString::FindFirst(const char c) const
{
	length_type i = 0;
	for(auto& e : data)
	{
		if (e == c)	{ return i; }

		++i;
	}

	return npos();
}

void FString::Drop(int64 from)
{
	data.resize(from + 1);
	data[from + 1] = '\0';
}

void FString::ReplaceAll(char a, char with)
{
	for (uint32 i = 0; i < data.getLength() - 1; ++i)
	{
		if (data[i] == a)
		{
			data[i] = with;
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

		while(ocurrences.getLength() < ocurrences.getCapacity() && i < data.getLength() - 1) //while we don't exceed the occurrences array capacity and we are not at the end of the array(because we might hit the end in the first caching iteration)
		{
			if (data [i] == a[0]) //if current char matches the a's first character enter whole word loop check
			{
				uint32 j = 1;
				
				for (; j < a_length; ++j) //if the a text is matched add occurrence else quickly escape loop and go to next whole string loop
				{
					if (data[i + j] != a[j]) 
					{
						break;
					}
				}

				if (j == a_length - 1) //if loop found word make_space occurrence and jump i by a's length
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
			data.resize(data.getLength() + resize_size);
		}

		for (auto& e : ocurrences)
		{
			data.make_space(e, with_length - a_length);
			data.overwrite(with_length, const_cast<string_type*>(with), e);
		}

		if (i == data.getLength() - 1) //if current index is last index in whole string break out of the loop
		{
			break;
		}
	}
}

constexpr FString::length_type FString::StringLength(const char* In)
{
	length_type length = 0;

	while (In[length] != '\0') { length++; }

	//We return Length + 1 to take into account for the null terminator character.
	return length + 1;
}

#define FSTRING_MAKESTRING_DEFAULT_SIZE 256

FString FString::MakeString(const char* _Text, ...)
{
	FString Return(FSTRING_MAKESTRING_DEFAULT_SIZE);

	va_list vaargs;
	va_start(vaargs, _Text);
	const auto Count = snprintf(Return.data.getData(), Return.data.getLength(), _Text, vaargs) + 1;
	//Take into account null terminator.
	if (Count > Return.data.getLength())
	{
		Return.data.resize(Count);

		snprintf(Return.data.getData(), Return.data.getLength(), _Text, vaargs);
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
