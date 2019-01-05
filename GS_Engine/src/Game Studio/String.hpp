#pragma once

#include "Core.h"

#include "DArray.hpp"

#include <cstring>

GS_CLASS String
{
public:
	String(const char * In);
	~String();

	void operator=(const char *);

	const char * c_str();
	unsigned int GetLength() const { return Arrayptr->GetArrayLength(); }
	bool IsEmpty() const { return Arrayptr->GetArrayLength() == 0; }
private:
	DArray<char> * Arrayptr;
};


String::String(const char * In)
{
	unsigned short TextLength = strlen(In) + 1;

	Arrayptr = new DArray<char>(TextLength);

	for (unsigned int i = 0; i < TextLength; i++)
	{
		(* Arrayptr)[i] = In[i];
	}
}

String::~String()
{
	delete Arrayptr;
}

void String::operator=(const char * In)
{
	for (unsigned int i = 0; i < strlen(In) + 1; i++)
	{
		(* Arrayptr)[i] = In[i];
	}
}

const char * String::c_str()
{
	return Arrayptr->GetArrayPointer();
}