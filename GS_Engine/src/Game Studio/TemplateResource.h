#pragma once

#include "Core.h"

#include <string>

template <typename T>
GS_CLASS TemplateResource : public Resource
{
public:
	//Returns a pointer to the data.
	T * GetData() const { return Data; };

protected:

	T * Data = nullptr;

	//Resource identifier. Used to check if a resource has already been loaded.
	std::string Path;

	//Size in bytes of the allocated memory for this resource.
	size_t Size = 0;
};