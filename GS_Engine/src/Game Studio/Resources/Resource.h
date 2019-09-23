#pragma once

#include "Core.h"

#include "Containers/FString.h"

//Base class representation of all types of resources that can be loaded into the engine.
class GS_API Resource
{
public:
	Resource() = default;

	Resource(const FString & Path) : FilePath(Path)
	{
	}

	virtual ~Resource() = default;


	//Returns the size of the data.
	[[nodiscard]] virtual size_t GetDataSize() const = 0;

	[[nodiscard]] const FString& GetPath() const { return FilePath; }

	void IncrementReferences() { ++References; }
	void DecrementReferences() { --References; }
	uint16 GetReferenceCount() const { return References; }

protected:
	void* Data = nullptr;

	uint16 References = 0;

	//Resource identifier. Used to check if a resource has already been loaded.
	FString FilePath;

	virtual bool LoadResource() = 0;
	virtual void LoadFallbackResource() = 0;
};