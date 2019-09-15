#pragma once

#include "Core.h"

#include "FVector.hpp"

GS_CLASS FString
{
public:
	//Constructs an empty FString.
	FString();

	template<size_t N>
	FString(const char(&_Literal)[N]) : Data(_Literal, N)
	{
	}

	explicit FString(char* const _In);

	//Constructs an FString from a C-FString.
	explicit FString(const char * In);

	explicit FString(size_t _Length);

	//Constructs a FString from a non null terminated character array.
	FString(const char * In, size_t Length);

	FString(const FString & Other) = default;

	~FString() = default;

	FString & operator=(const char *);
	FString & operator=(const FString & Other) = default;
	FString & operator+(const char * Other);
	FString & operator+(const FString & Other);

	//Returns true if the two FString's contents are the same. Comparison is case sensitive.
	bool operator==(const FString & Other) const;

	//Returns true if the two FString's contents are the same. Comparison is case insensitive.
	[[nodiscard]] bool NonSensitiveComp(const FString& _Other) const;

	//Returns the contents of this FString as a C-FString.
	char * c_str();

	//Returns the contents of this FString as a C-FString.
	[[nodiscard]] const char * c_str() const;

	//Return the length of this FString. Does not take into account the null terminator character.
	INLINE size_t GetLength() const { return Data.length() - 1; }
	//Returns whether this FString is empty.
	INLINE bool IsEmpty() const { return Data.length() == 0; }

	//Places a the C-FString after this FString with a space in the middle.
	void Append(const char * In);
	//Places the FString after this FString with a space in the middle.
	void Append(const FString & In);

	//Places the passed in FString at the specified Index.
	void Insert(const char * In, size_t Index);

	//Returns the index to the last character in the string that is equal to _Char, if no matching character is found -1 is returned.
	[[nodiscard]] int64 FindLast(char _Char) const;

	//Returns the length of the In FString accounting for the null terminator character. FString MUST BE NULL TERMINATED.
	static size_t StringLength(const char * In);

private:
	FVector<char> Data;

	static FString MakeString(const char* _Text, ...);
	static char ToLowerCase(char _Char);
	static char ToUpperCase(char _Char);
};