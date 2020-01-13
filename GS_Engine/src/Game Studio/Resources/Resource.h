#pragma once

#include "Core.h"

#include "Object.h"

#include "Containers/FString.h"

using ResourceHeaderType = uint64;
using ResourceSegmentType = uint64;

class ResourceData
{
	friend class Resource;

public:
	FString ResourceName;
	
	ResourceData() = default;

	virtual ~ResourceData()
	{
	}

	virtual void** WriteTo(size_t _Index, size_t _Bytes) = 0;

	virtual void Load(InStream& InStream_);
	virtual void Write(OutStream& OutStream_);
	
	const FString& GetResourceName() const { return ResourceName; }
};

/**
 * \brief Base class representation of all types of resources that can be loaded into the engine.
 */
class GS_API Resource : public Object
{
	friend class ResourceManager;
	
	uint16 References = 0;
	
	void IncrementReferences() { ++References; }
	void DecrementReferences() { --References; }
	[[nodiscard]] uint16 GetReferenceCount() const { return References; }
	
	virtual bool LoadResource(const FString& _FullPath) = 0;
	virtual void LoadFallbackResource(const FString& _FullPath) = 0;
	
	//Must return the extension name for the extension type.
	[[nodiscard]] virtual const char* GetResourceTypeExtension() const = 0;
	
public:
	Resource() = default;

	virtual ~Resource() = default;
};

struct ResourceElementDescriptor
{
	uint64 Bytes = 0;
	//void* Data = nullptr;
};

struct SaveResourceElementDescriptor
{
	SaveResourceElementDescriptor(ResourceSegmentType _Bytes, void* _Data) : Bytes(_Bytes), Data(_Data)
	{
	}

	ResourceSegmentType Bytes = 0;
	void* Data = nullptr;
};