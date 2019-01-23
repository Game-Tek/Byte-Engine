#pragma once

#include "Core.h"

template <typename T>
GS_CLASS Resource
{
public:
	//Returns a pointer to the data.
	T * GetData() const { return Data; };

protected:
	T * Data;

	//Resource identifier. Used to check if a resource has already been loaded.
	unsigned int ResourceId = 0;

	//Size in bytes of the allocated memory for this resource.
	size_t Size = 0;

	//When overriden this function should provide a pointer to the memory region where it loaded all the data.
	virtual T * Load(const char * FilePath) = 0;

	virtual T * LoadFallbackResource() = 0;
};