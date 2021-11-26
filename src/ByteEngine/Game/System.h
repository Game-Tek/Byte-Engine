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

	struct InitializeInfo {
		class ApplicationManager* GameInstance{ nullptr };
		/**
		 * \brief Rough estimate for number of components present during average run of the application.
		 * Can be used for initialization of data structures to allocate "enough" space during start as to avoid as many re-allocations further down the line.
		 */
		uint32 ScalingFactor = 0;
		uint16 SystemId;
		Id InstanceName;

		ApplicationManager* operator->() { return GameInstance; }
	};	
	System(const InitializeInfo& initializeInfo, const utf8* name) : Object(name), systemId(initializeInfo.SystemId), instanceName(initializeInfo.InstanceName)
	{
	}

	//struct ShutdownInfo
	//{
	//	class ApplicationManager* GameInstance = nullptr;
	//};
	//void Shutdown(const ShutdownInfo& shutdownInfo);

	[[nodiscard]] uint16 GetSystemId() const { return systemId; }

protected:
	
private:
	uint16 systemId;
	Id instanceName;

	friend class ApplicationManager;
};
