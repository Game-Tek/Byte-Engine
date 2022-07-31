#pragma once

#include "ByteEngine/Object.h"

#include "ByteEngine/Debug/Assert.h"

namespace GTSL {
	template<typename T, class ALLOC>
	class Vector;
}

template<typename T, class ALLOC = BE::PersistentAllocatorReference>
using Vector = GTSL::Vector<T, ALLOC>;

class ApplicationManager;

namespace BE {
	/**
	 * \brief Systems persist across levels and can process world components regardless of the current level.
	 * Used to instantiate render engines, sound engines, physics engines, AI systems, etc.
	 */
	class System : public Object {
	public:
		struct InitializeInfo {
			ApplicationManager* ApplicationManager{ nullptr };
			/**
			 * \brief Rough estimate for number of components present during average run of the application.
			 * Can be used for initialization of data structures to allocate "enough" space during start as to avoid as many re-allocations further down the line.
			 */
			uint32 ScalingFactor = 0;
			uint16 SystemId;
			Id InstanceName;
		};
		System(const InitializeInfo& initializeInfo, const utf8* name) : Object(name), systemId(initializeInfo.SystemId), instanceName(initializeInfo.InstanceName), application_manager_(initializeInfo.ApplicationManager)
		{
		}

		//struct ShutdownInfo
		//{
		//	class ApplicationManager* ApplicationManager = nullptr;
		//};
		//void Shutdown(const ShutdownInfo& shutdownInfo);

		[[nodiscard]] uint16 GetSystemId() const { return systemId; }

	protected:

		ApplicationManager* GetApplicationManager() const { return application_manager_; }

	private:
		uint16 systemId;
		Id instanceName;

		ApplicationManager* application_manager_ = nullptr;

		friend class ApplicationManager;
	};
}