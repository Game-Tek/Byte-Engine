#pragma once

#include "Core.h"

#include <string>

GS_CLASS Resource
{
public:
	//Returns a pointer to the data.
	void * GetData() const { return Data; }

	//Returns the size of the data.
	virtual size_t GetDataSize() const = 0;

	const std::string &  GetPath() { return Path; }

protected:
	//Pointer to the data owned by this resource;
	void * Data;

	//Resource identifier. Used to check if a resource has already been loaded.
	std::string Path;
};