#pragma once

#include "Core.h"

#include <cstring>
#include "FVector.hpp"

GS_CLASS String
{
public:
	String();
	String(const char * In);
	String(const char* In, size_t Length);
	~String();

	String & operator=(const char *);
	String & operator=(const String & Other);

	const char * c_str();
	INLINE unsigned int GetLength() const { return Array.length(); }
	INLINE bool IsEmpty() const { return Array.length() == 0; }
private:
	FVector<char> Array;
};
