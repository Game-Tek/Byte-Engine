#include "FString.h"
#include <cstdio>
#include <cstdarg>

#include "Resources/Resource.h"
#include "Array.hpp"
#include "Ranger.h"

OutStream& operator<<(OutStream& archive, FString& string)
{
	archive << string.data;

	return archive;
}

InStream& operator>>(InStream& archive, FString& string)
{
	archive >> string.data;

	return archive;
}

FString::FString() : data(10)
{
}

FString& FString::operator=(const char* cstring)
{
	data.recreate(StringLength(cstring), const_cast<char*>(cstring));
	return *this;
}

FString& FString::operator+=(char c)
{
	data.pop_back();
	data.push_back(c);
	data.push_back('\0');
	return *this;
}

FString& FString::operator+=(const char* cstring)
{
	data.pop_back();
	data.push_back(StringLength(cstring), cstring);
	return *this;
}

FString& FString::operator+=(const FString& string)
{
	data.pop_back(); data.push_back(string.data); return *this;
}

bool FString::operator==(const FString& other) const
{
	//Discard if Length of strings is not equal, first because it helps us discard before even starting, second because we can't compare strings of different sizes.
	if (data.getLength() != other.data.getLength()) return false;

	length_type i = 0;
	for (const auto& c : data) { if (c != other.data[i]) { return false; } ++i; }
	return true;
}

bool FString::NonSensitiveComp(const FString& other) const
{
	//Discard if Length of strings is not equal, first because it helps us discard before even starting, second because we can't compare strings of different sizes.
	if (data.getLength() != other.data.getLength()) return false;

	length_type i = 0;
	for (const auto& c : data) { if (c != (toLowerCase(other.data[i]) || toUpperCase(other.data[i]))) { return false; } ++i; }

	return true;
}

void FString::Append(const char* cstring)
{
	data.pop_back(); //Get rid of null terminator.
	data.push_back(' '); //Push space.
	data.push_back(StringLength(cstring), const_cast<char*>(cstring));
	return;
}

void FString::Append(const FString& string)
{
	data.pop_back(); //Get rid of null terminator.
	data.push_back(' '); //Push space.
	data.push_back(string.data); //Push new string.
	return;
}

void FString::Append(const uint8 number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 3 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%d", number) + 1);
}

void FString::Append(const int8 number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 4 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%d", number) + 1);
}

void FString::Append(const uint16 number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 6 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%hu", number) + 1);
}

void FString::Append(const int16 number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 7 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%hi", number) + 1);
}

void FString::Append(const uint32 number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 10 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%lu", number) + 1);
}

void FString::Append(const int32 number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 11 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%d", number) + 1);
}

void FString::Append(const uint64 number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 20 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%llu", number) + 1);
}

void FString::Append(const int64 number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 21 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%lld", number) + 1);
}

void FString::Append(const float number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 31 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%f", number) + 1);
}

void FString::Append(const double number)
{
	data.pop_back();
	data.push_back(' ');
	data.resize(data.getLength() + 61 + 1);
	data.resize(sprintf_s(data.getData() + data.getLength() - 1, data.getCapacity() - data.getLength() - 1, "%lf", number) + 1);
}

void FString::Insert(const char* cstring, const length_type index)
{
	data.push(index, const_cast<char*>(cstring), StringLength(cstring));
	return;
}

FString::length_type FString::FindFirst(const char c) const
{
	length_type i = 0;
	for (const auto& e : data) { if (e == c) { return i; } ++i; }
	return npos();
}

FString::length_type FString::FindLast(const char c) const
{
	length_type i = 0;
	for (auto& e : Ranger(data.end(), data.begin())) { if (e == c) { return i; } ++i; }
	return npos();
}

void FString::Drop(const length_type from)
{
	data.resize(from + 1);
	data[from + 1] = '\0';
}

void FString::ReplaceAll(const char a, const char with)
{
	for (auto& c : data) { if (c == a) { c = with; } }
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

constexpr FString::length_type FString::StringLength(const char* cstring)
{
	length_type length = 0;	while (*cstring) { ++length; }

	//We return Length + 1 to take into account for the null terminator character.
	return length + 1;
}

#define FSTRING_MAKESTRING_DEFAULT_SIZE 256

FString FString::MakeString(const char* cstring, ...)
{
	FString Return(FSTRING_MAKESTRING_DEFAULT_SIZE);

	va_list vaargs;
	va_start(vaargs, cstring);
	const auto Count = snprintf(Return.data.getData(), Return.data.getLength(), cstring, vaargs) + 1;
	//Take into account null terminator.
	if (Count > Return.data.getLength())
	{
		Return.data.resize(Count);

		snprintf(Return.data.getData(), Return.data.getLength(), cstring, vaargs);
	}
	va_end(vaargs);

	return Return;
}

char FString::toLowerCase(char c)
{
	if ('A' <= c && c <= 'Z') return c += ('a' - 'A');
	return c;
}

char FString::toUpperCase(char c)
{
	if ('a' <= c && c <= 'z') return c += ('a' - 'A');
	return c;
}
