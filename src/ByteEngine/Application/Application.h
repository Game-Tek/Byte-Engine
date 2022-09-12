#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Allocator.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/String.hpp>
#include <GTSL/Application.h>

#include "Clock.h"
#include "PoolAllocator.h"
#include "StackAllocator.h"
#include "SystemAllocator.h"
#include "ByteEngine/Id.h"
#include "ByteEngine/Resources/ResourceManager.h"

class ApplicationManager;
class InputManager;
class ThreadPool;
class Clock;

#undef ERROR

#include <GTSL/JSON.hpp>

namespace BE
{	
	class Logger;
	
	class Application : public Object {
	public:
		[[nodiscard]] static const char* GetEngineName() { return "Byte Engine"; }
		static const char* GetEngineVersion() { return "0.0.1"; }

		static Application* Get() { return applicationInstance; }
		
		explicit Application(GTSL::ShortString<128> applicationName);
		virtual ~Application();

		bool BaseInitialize(int argc, utf8* argv[]);
		virtual bool Initialize() = 0;
		virtual void PostInitialize() = 0;
		virtual void Shutdown() = 0;
		uint8 GetNumberOfThreads();

		enum class UpdateContext : uint8 {
			NORMAL, BACKGROUND
		};
		
		struct OnUpdateInfo {};
		virtual void OnUpdate(const OnUpdateInfo& updateInfo);
		
		int Run(int argc, char** argv);
		
		virtual GTSL::ShortString<128> GetApplicationName() = 0;

		//Fires a Delegate to signal that the application has been requested to close.
		void PromptClose();

		enum class CloseMode : uint8 {
			OK, WARNING, ERROR
		};
		//Flags the application to close on the next update.
		void Close(CloseMode closeMode, GTSL::Range<const utf8*> reason);

		//Immediately closes the application and logs the reason
		void Exit(const GTSL::Range<const utf8*> reason) {
			GTSL::Lock lock(crashLogMutex);
			
			if(!crashLog) {
				crashLog.Open(GTSL::ShortString<32>(u8"crash.log"), GTSL::File::WRITE, true);
			}

			GTSL::Buffer<GTSL::StaticAllocator<512>> buffer;
			GTSL::Insert(reason, buffer);
			crashLog.Write(buffer);

			//todo: open dialog box?
		}
		
		[[nodiscard]] GTSL::StaticString<260> GetPathToApplication() const;

//		[[nodiscard]] GTSL::StaticString<260> GetPathToApplication() const {
	//		auto path = systemApplication.GetPathToExecutable();
		//	path.Drop(FindLast(path, u8'/').Get()); return path;
		//}
		
		[[nodiscard]] const Clock* GetClock() const { return &clockInstance; }
		[[nodiscard]] InputManager* GetInputManager() const { return inputManagerInstance.GetData(); }
		[[nodiscard]] Logger* GetLogger() const { return logger.GetData(); }
		[[nodiscard]] const GTSL::Application* GetSystemApplication() const { return &systemApplication; }
		[[nodiscard]] class ApplicationManager* GetGameInstance() const { return applicationManager; }
		
		[[nodiscard]] uint64 GetApplicationTicks() const { return applicationTicks; }
		
		[[nodiscard]] ThreadPool* GetThreadPool() const { return threadPool; }
		
		[[nodiscard]] SystemAllocator* GetSystemAllocator() { return &systemAllocator; }
		[[nodiscard]] PoolAllocator* GetPersistantAllocator() { return &poolAllocator; }
		[[nodiscard]] StackAllocator* GetTransientAllocator() { return &transientAllocator; }

		bool GetBoolOption(const GTSL::StringView optionName) const {
			return JSON[optionName].GetBool();
		}

		uint64 GetUINTOption(const GTSL::StringView optionName) const {
			return JSON[optionName].GetUint();
		}

		GTSL::StringView GetStringOption(const GTSL::StringView optionName) const {
			return JSON[optionName].GetStringView();
		}

		GTSL::Extent2D GetExtent2DOption(const GTSL::StringView optionName) const {
			return { static_cast<uint16>(JSON[optionName][0].GetUint()), static_cast<uint16>(JSON[optionName][1].GetUint()) };
		}

		const auto& GetConfig() const {
			return JSON;
		}
		
	protected:
		inline static Application* applicationInstance{ nullptr };
		
		SystemAllocator systemAllocator;
		SystemAllocatorReference systemAllocatorReference;

		GTSL::File crashLog;
		GTSL::Mutex crashLogMutex;
		
		GTSL::SmartPointer<Logger, SystemAllocatorReference> logger;
		GTSL::SmartPointer<ApplicationManager, BE::SystemAllocatorReference> applicationManager;
		
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

		GTSL::Buffer<BE::PAR> jsonBuffer;
		GTSL::JSON<GTSL::DefaultAllocatorReference> JSON;

		bool parseConfig();
		/**
		 * \brief Checks if the platform (OS, CPU, RAM) satisfies certain requirements specified for the program.
		 * \return Whether the current platform supports the required features.
		 */
		bool checkPlatformSupport();
	private:
	};

	Application* CreateApplication(GTSL::AllocatorReference* allocatorReference);
	void DestroyApplication(Application* application, GTSL::AllocatorReference* allocatorReference);
}