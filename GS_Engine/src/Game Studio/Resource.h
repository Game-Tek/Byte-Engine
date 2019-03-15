#pragma once

#include "Core.h"

#include "String.h"

//Base class representation of all types of resources that can be loaded into the engine.
GS_CLASS Resource
{
public:
	Resource() = default;
	Resource(const String & Path) : FilePath(Path)
	{
	}
	virtual ~Resource() = default;

	//Returns a pointer to the data.
	void * GetData() const { return Data; }

	//Returns the size of the data.
	virtual size_t GetDataSize() const = 0;

	const String & GetPath() const { return FilePath; }

protected:
	//Pointer to the data owned by this resource;
	void * Data = nullptr;

	//Resource identifier. Used to check if a resource has already been loaded.
	String FilePath;
};