#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Application.h>
#include <GTSL/Allocator.h>
#include <GTSL/HashMap.h>
#include <GTSL/String.hpp>


#include "Clock.h"
#include "PoolAllocator.h"
#include "StackAllocator.h"
#include "SystemAllocator.h"
#include "ByteEngine/Id.h"

class ResourceManager;
class GameInstance;
class InputManager;
class ThreadPool;
class Clock;

#undef ERROR

namespace BE
{	
	class Logger;
	
	/**
	 * \brief Defines all the data necessary to startup a GameStudio application instance.
	 */
	struct ApplicationCreateInfo
	{
		const char* ApplicationName = nullptr;
	};
	
	class Application : public Object
	{
	public:
		[[nodiscard]] static const char* GetEngineName() { return "Byte Engine"; }
		static const char* GetEngineVersion() { return "0.0.1"; }

		static Application* Get() { return applicationInstance; }
		
		explicit Application(const ApplicationCreateInfo& ACI);
		virtual ~Application();

		void SetSystemAllocator(SystemAllocator* newSystemAllocator) { systemAllocator = newSystemAllocator; }

		bool BaseInitialize(int argc, utf8* argv[]);
		virtual bool Initialize() = 0;
		virtual void PostInitialize() = 0;
		virtual void Shutdown() = 0;
		uint8 GetNumberOfThreads();

		enum class UpdateContext : uint8
		{
			NORMAL, BACKGROUND
		};
		
		struct OnUpdateInfo
		{
		};
		virtual void OnUpdate(const OnUpdateInfo& updateInfo);
		
		int Run(int argc, char** argv);
		
		virtual const char* GetApplicationName() = 0;

		//Fires a Delegate to signal that the application has been requested to close.
		void PromptClose();

		enum class CloseMode : uint8
		{
			OK, WARNING, ERROR
		};
		//Flags the application to close on the next update.
		void Close(CloseMode closeMode, const GTSL::Range<const utf8*> reason);

		[[nodiscard]] GTSL::StaticString<260> GetPathToApplication() const
		{
			auto path = systemApplication.GetPathToExecutable();
			path.Drop(path.FindLast('/').Get().Second); return path;
		}
		
		[[nodiscard]] const Clock* GetClock() const { return &clockInstance; }
		[[nodiscard]] InputManager* GetInputManager() const { return inputManagerInstance.GetData(); }
		[[nodiscard]] Logger* GetLogger() const { return logger.GetData(); }
		[[nodiscard]] const GTSL::Application* GetSystemApplication() const { return &systemApplication; }
		[[nodiscard]] class GameInstance* GetGameInstance() const { return gameInstance; }

		template<typename RM>
		RM* CreateResourceManager()
		{
			auto resource_manager = GTSL::SmartPointer<ResourceManager, BE::SystemAllocatorReference>::Create<RM>(systemAllocatorReference);
			return static_cast<RM*>(resourceManagers.Emplace(GTSL::Id64(resource_manager->GetName()), MoveRef(resource_manager)).GetData());
		}
		
		[[nodiscard]] uint64 GetApplicationTicks() const { return applicationTicks; }
		
		template<class T>
		T* GetResourceManager(const Id name) { return static_cast<T*>(resourceManagers.At(name).GetData()); }
		
		[[nodiscard]] ThreadPool* GetThreadPool() const { return threadPool; }
		
		[[nodiscard]] SystemAllocator* GetSystemAllocator() const { return systemAllocator; }
		[[nodiscard]] PoolAllocator* GetNormalAllocator() { return &poolAllocator; }
		[[nodiscard]] StackAllocator* GetTransientAllocator() { return &transientAllocator; }

		uint32 GetOption(const Id name) const
		{
			return settings.At(name);
		}
		
	protected:
		GTSL::SmartPointer<Logger, SystemAllocatorReference> logger;
		GTSL::SmartPointer<GameInstance, SystemAllocatorReference> gameInstance;

		GTSL::HashMap<Id, GTSL::SmartPointer<ResourceManager, SystemAllocatorReference>, SystemAllocatorReference> resourceManagers;

		GTSL::HashMap<Id, uint32, PersistentAllocatorReference> settings;
		
		SystemAllocatorReference systemAllocatorReference;
		SystemAllocator* systemAllocator{ nullptr };
		PoolAllocator poolAllocator;
		StackAllocator transientAllocator;

		GTSL::Application systemApplication;

		bool initialized = false;
		
		Clock clockInstance;
		GTSL::SmartPointer<InputManager, BE::SystemAllocatorReference> inputManagerInstance;
		GTSL::SmartPointer<ThreadPool, BE::SystemAllocatorReference> threadPool;

		bool flaggedForClose = false;
		CloseMode closeMode{ CloseMode::OK };
		BE_DEBUG_ONLY(GTSL::StaticString<1024> closeReason);

		uint64 applicationTicks{ 0 };

		bool parseConfig();
		/**
		 * \brief Checks if the platform (OS, CPU, RAM) satisfies certain requirements specified for the program.
		 * \return Whether the current platform supports the required features.
		 */
		bool checkPlatformSupport();
	private:
		inline static Application* applicationInstance{ nullptr };
	};

	Application* CreateApplication(GTSL::AllocatorReference* allocatorReference);
	void DestroyApplication(Application* application, GTSL::AllocatorReference* allocatorReference);
}
