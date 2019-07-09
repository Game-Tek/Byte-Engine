#pragma once

#include "Core.h"

#include "FVector.hpp"

GS_CLASS String
{
public:
	//Constructs an empty String.
	String();

	//Constructs an String from a C-String.
	explicit String(const char * In);

	String(const String & Other);

	//Constructs a String from a non null terminated character array.
	String(const char * In, size_t Length);

	~String() = default;

	String & operator=(const char *);
	String & operator=(const String & Other);
	String & operator+(const char * Other);
	String & operator+(const String & Other);

	bool operator==(const String & Other) const;

	//Returns the contents of this string as a C-String.
	char * c_str();

	//Returns the contents of this string as a C-String.
	const char * c_str() const;

	//Return the length of this string. Does not take into account the null terminator character.
	INLINE size_t GetLength() const { return Array.length() - 1; }
	//Returns whether this string is empty.
	INLINE bool IsEmpty() const { return Array.length() == 0; }

	//Places a the C-String after this string with a space in the middle.
	void Append(const char * In);
	//Places the string after this string with a space in the middle.
	void Append(const String & In);

	//Places the passed in string at the specified Index.
	void Insert(const char * In, size_t Index);

	//Returns the length of the In string accounting for the null terminator character. STRING MUST BE NULL TERMINATED.
	static size_t StringLength(const char * In);

private:
	FVector<char> Array;
};
