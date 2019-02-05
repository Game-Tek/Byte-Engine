#pragma once

#include "Core.h"

#include <string>

GS_CLASS Resource
{
public:
	//Returns a pointer to the data.
	void * GetData() const { return Data; };
	const std::string &  GetPath() { return Path; }

protected:

	void * Data;

	//Resource identifier. Used to check if a resource has already been loaded.
	std::string Path;

	//Size in bytes of the allocated memory for this resource.
	size_t Size = 0;
};