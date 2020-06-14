#pragma once

#include "Application/AllocatorReferences.h"

/**
 * \brief Base class for most non-data only classes in the engine.
 */
class Object
{
public:
	Object() = default;
	virtual ~Object() = default;

	[[nodiscard]] virtual const char* GetName() const = 0;

	[[nodiscard]] BE::PersistentAllocatorReference GetPersistentAllocator() const
	{
		return BE::PersistentAllocatorReference(GetName());
	}

	[[nodiscard]] BE::TransientAllocatorReference	 GetTransientAllocator() const
	{
		return BE::TransientAllocatorReference(GetName());
	}
};
