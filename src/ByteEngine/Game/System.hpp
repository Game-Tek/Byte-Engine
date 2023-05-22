#pragma once

#include "ByteEngine/Object.h"
#include "ByteEngine/Debug/Assert.h"

namespace GTSL
{
	template<typename T, class ALLOC>
	class Vector;
}

template<typename T,class ALLOC = BE::PersistentAllocatorReference>
using Vector = GTSL::Vector<T, ALLOC>;

class ApplicationManager;

namespace BE
{
	/*
	 * \brief Systems persist across levels and can process world components.
	 * Used to instantiate systems such as rendering, audio, physics, AI, etc.
	 */
	class System : public Object
	{
	public:
		struct InitializeInfo
		{
			ApplicationManager* AppManager = nullptr;
			/**
			 * \brief Rough estimate for number of components present during average run of the application.
			 * Can be used for initialization of data structures to allocate "enough" space during start as to avoid as many re-allocations further down the line.
			 */
			GTSL::uint32 ScalingFactor = 0;
			GTSL::uint16 SystemId;
			Id InstanceName;
		};

		System(const InitializeInfo& info, const char8_t* name)
			: Object(name), m_systemId(info.SystemId)
				,m_instanceName(info.InstanceName), m_appManager(info.AppManager)
		{}

		[[nodicard]] GTSL::uint16 GetSystemId() const { return m_systemId; }
	protected:
		ApplicationManager* GetApplicationManager() const { return m_appManager; }
	private:
		friend ApplicationManager;

		GTSL::uint16 m_systemId;
		Id m_instanceName;
		ApplicationManager* m_appManager = nullptr;
	};
}