#pragma once

#include "Core.h"

GS_CLASS Resource
{
public:
	//Returns a pointer to the data.
	void * GetData() const { return Data; }

protected:
	//Resource identifier. Used to check if a resource has already been loaded.
	unsigned int ResourceId = 0;

	//Pointer to the data represented by this resource.
	void * Data = nullptr;

	//Size in bytes of the allocated memory for this resource.
	size_t Size = 0;

	//When overriden this function should provide a pointer to the memory region where it loaded all the data.
	virtual void * Load(const char * FilePath) = 0;
};