#pragma once

#include "Core.h"

#include "GSArray.hpp"

GS_CLASS String
{
public:
	String(const char * In);
	~String();

private:
	GArray<char> * Arrayptr;
};


String::String(const char * In)
{
	Arrayptr = new GArray<char>(5);
}

String::~String()
{
	delete Arrayptr;
}