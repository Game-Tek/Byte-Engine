#pragma once

#include "Application/AllocatorReferences.h"

/**
 * \brief Base class for most non-data only classes in the engine.
 */
class Object
{
public:
	Object() = default;
	
	Object(const UTF8* objectName) : name(objectName) {}
	
	~Object() = default;

	[[nodiscard]] const char* GetName() const { return name; }

	[[nodiscard]] BE::PersistentAllocatorReference GetPersistentAllocator() const
	{
		return BE::PersistentAllocatorReference(GetName());
	}

	[[nodiscard]] BE::TransientAllocatorReference	 GetTransientAllocator() const
	{
		return BE::TransientAllocatorReference(GetName());
	}

private:
	const UTF8* name = "Object";
};
