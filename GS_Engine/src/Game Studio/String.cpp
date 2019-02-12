#include "String.hpp"

String::String() : Array(10)
{
}

String::String(const char * In) : Array(const_cast<char *>(In), strlen(In) + 1)
{
}

String::String(const char * In, const size_t Length) : Array(const_cast<char *>(In), Length)
{
}

String::~String()
{
}

String & String::operator=(const char * In)
{
	Array.overlay(0, const_cast<char *>(In), strlen(In) + 1);

	return *this;
}

String & String::operator=(const String & Other)
{
	Array = Other.Array;

	return *this;
}

const char * String::c_str()
{
	return Array.data();
}