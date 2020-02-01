#pragma once

#include "Core.h"

#include "Object.h"

#include "Containers/FString.h"
#include "Containers/Id.h"

#include "Stream.h"

using ResourceHeaderType = uint64;
using ResourceSegmentType = uint64;

template <typename T>
void SerializeFVector(OutStream& outStream, FVector<T>& vector)
{
	outStream.Write(vector.getLength());

	for (uint_64 i = 0; i < vector.getLength(); ++i)
	{
		outStream << vector[i];
	}
}

template <typename T>
void operator<<(OutStream& outStream, FVector<T>& vector)
{
	outStream.Write(vector.getLength());

	for (uint_64 i = 0; i < vector.getLength(); ++i)
	{
		outStream << vector[i];
	}
}

template <typename T>
void operator>>(InStream& inStream, FVector<T>& vector)
{
	typename FVector<T>::length_type length = 0;

	inStream.Read(&length);

	vector.forceRealloc(length);
	vector.resize(length);

	for (uint_64 i = 0; i < length; ++i)
	{
		inStream >> vector[i];
	}
}

template <typename T>
void DeserializeFVector(InStream& inStream, FVector<T>& vector)
{
	typename FVector<T>::length_type length = 0;

	inStream.Read(&length);

	vector.resize(length);

	for (uint_64 i = 0; i < length; ++i)
	{
		inStream >> vector[i];
	}
}

class ResourceData
{
	friend class Resource;

public:
	FString ResourceName;

	ResourceData() = default;

	virtual ~ResourceData()
	{
	}

	const FString& GetResourceName() const { return ResourceName; }
};

struct LoadResourceData
{
	FString FullPath;
	class ResourceManager* Caller = nullptr;
};

/**
 * \brief Base class representation of all types of resources that can be loaded into the engine.
 */
class Resource : public Object
{
	friend class ResourceManager;

	Id resourceName;

	uint16 references = 0;

	void incrementReferences() { ++references; }
	void decrementReferences() { --references; }
	[[nodiscard]] uint16 getReferenceCount() const { return references; }

	virtual bool loadResource(const LoadResourceData& loadResourceData) = 0;
	virtual void makeFromData(ResourceData& resourceData) {}
	virtual void loadFallbackResource(const FString& fullPath) = 0;

	//Must return the extension name for the extension type.
	[[nodiscard]] virtual const char* getResourceTypeExtension() const = 0;

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
