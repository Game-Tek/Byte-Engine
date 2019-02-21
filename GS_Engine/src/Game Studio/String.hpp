#pragma once

#include "Core.h"

#include <cstring>
#include "FVector.hpp"

GS_CLASS String
{
public:
	//Constructs an empty String.
	String();

	//Constructs an String from a C-String.
	explicit String(const char * In);

	//Constructs a String from a non null terminated character array.
	String(const char * In, size_t Length);

	~String() = default;

	String & operator=(const char *);
	String & operator=(const String & Other);

	const char * c_str();
	INLINE size_t GetLength() const { return Array.length(); }
	INLINE bool IsEmpty() const { return Array.length() == 0; }

	void Append(const char * In);
	void Insert(const char * In, size_t Index);

private:
	FVector<char> Array;

	//Returns the length of the In string accounting for the null terminator character. STRING MUST BE NULL TERMINATED.
	static size_t StringLength(const char * In);
};
