#pragma once

#include "Core.h"

#include "Array.h"


GS_CLASS String
{
public:
	String(const char * In);

	~String();

private:
	Array<char> * Arrayptr;
};


String::String(const char * In)
{
	Arrayptr = new Array<char>(5);
}

String::~String()
{
	delete Arrayptr;
}