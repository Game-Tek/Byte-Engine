#pragma once

#include "Core.h"

#include "Object.h"

#include "Containers/FString.h"

using ResourceHeaderType = uint64;

class ResourceData
{
	char* ResourceName = nullptr;

public:
	ResourceData() = default;

	virtual ~ResourceData()
	{
		delete[] ResourceName;
	}

	virtual void** WriteTo(size_t _Index, size_t _Bytes) = 0;
	char*& GetResourceName() { return ResourceName; }
};

//Base class representation of all types of resources that can be loaded into the engine.

class GS_API Resource : public Object
{
public:
	Resource() = default;

	virtual ~Resource() = default;

	//Returns the size of the data.
	[[nodiscard]] virtual size_t GetDataSize() const = 0;

	void IncrementReferences() { ++References; }
	void DecrementReferences() { --References; }
	[[nodiscard]] uint16 GetReferenceCount() const { return References; }

	virtual bool LoadResource(const FString& _FullPath) = 0;
	virtual void LoadFallbackResource(const FString& _FullPath) = 0;

	ResourceData* GetData() const { return Data; }

	//Must return the extension name for the extension type, MUST contain the dot.
	//IE: ".gsasset". NOT "gsasset".
	[[nodiscard]] virtual const char* GetResourceTypeExtension() const = 0;

protected:
	ResourceData* Data = nullptr;

	uint16 References = 0;
};

struct ResourceElementDescriptor
{
	uint64 Bytes = 0;
	//void* Data = nullptr;
};

struct SaveResourceElementDescriptor
{
	uint64 Bytes = 0;
	void* Data = nullptr;
};