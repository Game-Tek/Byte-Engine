#pragma once

#include "ByteEngine/Object.h"
#include <GTSL/Id.h>

#include "ByteEngine/Debug/Assert.h"

namespace GTSL
{
	template<typename T, class ALLOC>
	class Vector;
}

template<typename T, class ALLOC = BE::PersistentAllocatorReference>
using Vector = GTSL::Vector<T, ALLOC>;

/**
 * \brief Systems persist across levels and can process world components regardless of the current level.
 * Used to instantiate render engines, sound engines, physics engines, AI systems, etc.
 */
class System : public Object
{
public:
	System() = default;
	
	System(const UTF8* name) : Object(name)
	{
	}

	struct InitializeInfo
	{
		class GameInstance* GameInstance{ nullptr };
		/**
		 * \brief Rough estimate for number of components present during average run of the application.
		 * Can be used for initialization of data structures to allocate "enough" space during start as to avoid as many re-allocations further down the line.
		 */
		uint32 ScalingFactor = 0;
	};
	virtual void Initialize(const InitializeInfo& initializeInfo) = 0;

	struct ShutdownInfo
	{
		class GameInstance* GameInstance = nullptr;
	};
	virtual void Shutdown(const ShutdownInfo& shutdownInfo) = 0;

	[[nodiscard]] uint16 GetSystemId() const { return systemId; }

protected:
	
private:
	uint16 systemId;

	friend class GameInstance;
};
