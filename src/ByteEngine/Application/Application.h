#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Allocator.hpp>
#include <GTSL/HashMap.hpp>
#include <GTSL/String.hpp>
#include <GTSL/Application.h>

#include "Clock.h"
#include "PoolAllocator.h"
#include "StackAllocator.h"
#include "SystemAllocator.h"
#include "ByteEngine/Id.h"
#include "ByteEngine/Resources/ResourceManager.h"
#include "GTSL/SmartPointer.hpp"

class ApplicationManager;
class InputManager;
class ThreadPool;
class Clock;

#undef ERROR

#include <GTSL/JSON.hpp>

class ShitTracker {
public:
	ShitTracker() {
		void* data  = nullptr;
		GTSL::Allocate(8ull, &data);
		++shitCount;
	}

	~ShitTracker() {
		--shitCount;
	}

	static uint32 shitCount;
};

namespace BE
{	
	class Logger;
	
	class Application : public Object {
	public:		
		explicit Application(GTSL::StringView applicationName);
		Application(const Application&) = delete; // Explicitly delete copy constructor because we have a static pointer to this object.
		Application(Application&&) = delete; // Explicitly delete move constructor because we have a static pointer to this object.
		virtual ~Application();

		[[nodiscard]] static const char* GetEngineName() { return "Byte Engine"; }
		static const char* GetEngineVersion() { return "0.0.1"; }

		static Application* Get() { return applicationInstance; }

		bool base_initialize(GTSL::Range<const GTSL::StringView*> arguments);

		virtual bool initialize() = 0;

		void run();

		enum class UpdateContext : uint8 {
			NORMAL, BACKGROUND
		};

		struct OnUpdateInfo {};
		virtual void OnUpdate(const OnUpdateInfo& updateInfo);

		virtual void shutdown() = 0;

		uint8 GetNumberOfThreads();	
		
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
			// GTSL::Lock lock(crashLogMutex);
			
			if(!crashLog) {
				auto crashLogFileOpenResult = crashLog.Open(GTSL::ShortString<32>(u8"crash.log"), GTSL::File::WRITE, true);
			}

			GTSL::Buffer<GTSL::StaticAllocator<512>> buffer;
			GTSL::Insert(reason, buffer);
			crashLog.Write(buffer);

			//todo: open dialog box?
		}
		
		[[nodiscard]] GTSL::StaticString<260> GetPathToApplication() const;
		
		//[[nodiscard]] const Clock* GetClock() const { return &clockInstance; }
		[[nodiscard]] const Clock* GetClock() const { return nullptr; }
		//[[nodiscard]] InputManager* GetInputManager() const { return inputManagerInstance.GetData(); }
		[[nodiscard]] InputManager* GetInputManager() const { return nullptr; }
		[[nodiscard]] Logger* GetLogger() const { return logger; }
		// [[nodiscard]] const GTSL::Application* GetSystemApplication() const { return &systemApplication; }
		[[nodiscard]] const GTSL::Application* GetSystemApplication() const { return nullptr; }
		[[nodiscard]] class ApplicationManager* GetGameInstance() const { return applicationManager; }
		[[nodiscard]] class ApplicationManager* get_orchestrator() const { return applicationManager; }
		
		[[nodiscard]] uint64 GetApplicationTicks() const { return applicationTicks; }
		
		[[nodiscard]] ThreadPool* GetThreadPool() const { return threadPool; }
		
		[[nodiscard]] SystemAllocator* GetSystemAllocator() { return &systemAllocator; }
		[[nodiscard]] PoolAllocator* GetPersistantAllocator() { return &poolAllocator; }
		[[nodiscard]] StackAllocator* GetTransientAllocator() { return &stackAllocator; }

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

		// GTSL::Application systemApplication;

		Clock clockInstance;

		GTSL::File crashLog;
		
		GTSL::SmartPointer<Logger, SystemAllocatorReference> logger;

		GTSL::SmartPointer<ApplicationManager, BE::SystemAllocatorReference> applicationManager;
		//ApplicationManager* applicationManager = nullptr;

		InputManager* inputManagerInstance = nullptr;

		PoolAllocator poolAllocator;
		StackAllocator stackAllocator;

		bool initialized = false;

		GTSL::SmartPointer<ThreadPool, BE::SystemAllocatorReference> threadPool;

		bool flaggedForClose = false;
		CloseMode closeMode{ CloseMode::OK };
		GTSL::StaticString<1024> closeReason;

		uint64 applicationTicks{ 0 };

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