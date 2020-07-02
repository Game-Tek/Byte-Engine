#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Application.h>
#include <GTSL/Allocator.h>
#include <GTSL/String.hpp>

#include "Clock.h"

#include "ByteEngine/Resources/ResourceManager.h"

#include "PoolAllocator.h"
#include "StackAllocator.h"
#include "SystemAllocator.h"

#include "EventManager.h"
#include "ByteEngine/Debug/Logger.h"

class InputManager;

class GameInstance;

namespace BE
{	
	/**
	 * \brief Defines all the data necessary to startup a GameStudio application instance.
	 */
	struct ApplicationCreateInfo
	{
		const char* ApplicationName = nullptr;
	};

//#undef ERROR
	
	class Application : public Object
	{
	public:	
		explicit Application(const ApplicationCreateInfo& ACI);
		virtual ~Application();

		void SetSystemAllocator(SystemAllocator* newSystemAllocator) { systemAllocator = newSystemAllocator; }

		virtual void Initialize() = 0;
		virtual void Shutdown() = 0;


		enum class UpdateContext : uint8
		{
			NORMAL, BACKGROUND
		};
		
		struct OnUpdateInfo
		{
			UpdateContext UpdateContext;
		};
		virtual void OnUpdate(const OnUpdateInfo& updateInfo);
		
		int Run(int argc, char** argv);
		
		virtual const char* GetApplicationName() = 0;
		[[nodiscard]] static const char* GetEngineName() { return "Byte Engine"; }
		static const char* GetEngineVersion() { return "0.0.1"; }

		static Application* Get() { return applicationInstance; }

		//Fires a Delegate to signal that the application has been requested to close.
		void PromptClose();

		enum class CloseMode : uint8
		{
			OK, ERROR
		};
		//Flags the application to close on the next update.
		void Close(CloseMode closeMode, const GTSL::Ranger<const UTF8>& reason);

		[[nodiscard]] const Clock* GetClock() const { return clockInstance; }
		[[nodiscard]] InputManager* GetInputManager() { return inputManagerInstance; }
		[[nodiscard]] ResourceManager* GetResourceManager() const { return resourceManagerInstance; }
		[[nodiscard]] EventManager* GetEventManager() { return &eventManager; }
		[[nodiscard]] Logger* GetLogger() const { return logger; }
		GTSL::Application* GetSystemApplication() { return &systemApplication; }
		class GameInstance* GetGameInstance() const { return gameInstance; }
		
		[[nodiscard]] uint64 GetApplicationTicks() const { return applicationTicks; }
		
		[[nodiscard]] SystemAllocator* GetSystemAllocator() const { return systemAllocator; }
		[[nodiscard]] PoolAllocator* GetNormalAllocator() const { return poolAllocator; }
		[[nodiscard]] StackAllocator* GetTransientAllocator() const { return transientAllocator; }

#ifdef BE_DEBUG
#define BE_LOG_SUCCESS(...)		BE::Application::Get()->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::SUCCESS, __VA_ARGS__);
#define BE_LOG_MESSAGE(...)		BE::Application::Get()->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::MESSAGE, __VA_ARGS__);
#define BE_LOG_WARNING(...)		BE::Application::Get()->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::WARNING, __VA_ARGS__);
#define BE_LOG_ERROR(...)		BE::Application::Get()->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::FATAL, __VA_ARGS__);
#define BE_LOG_LEVEL( Level)		BE::Application::Get()->GetLogger()->SetMinLogLevel(Level);

#define BE_BASIC_LOG_SUCCESS(...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::SUCCESS, __VA_ARGS__);
#define BE_BASIC_LOG_MESSAGE(...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::MESSAGE, __VA_ARGS__);
#define BE_BASIC_LOG_WARNING(...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::WARNING, __VA_ARGS__);
#define BE_BASIC_LOG_ERROR(...)		BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::FATAL, __VA_ARGS__);
#else
#define BE_LOG_SUCCESS(Text, ...)
#define BE_LOG_MESSAGE(Text, ...)
#define BE_LOG_WARNING(Text, ...)
#define BE_LOG_ERROR(Text, ...)
#define BE_LOG_LEVEL(_Level)
#define BE_BASIC_LOG_SUCCESS(Text, ...)	
#define BE_BASIC_LOG_MESSAGE(Text, ...)	
#define BE_BASIC_LOG_WARNING(Text, ...)	
#define BE_BASIC_LOG_ERROR(Text, ...)	
#endif

	protected:
		Logger* logger{ nullptr };

		GameInstance* gameInstance{nullptr};
		
		SystemAllocatorReference systemAllocatorReference;

		SystemAllocator* systemAllocator{ nullptr };
		PoolAllocator* poolAllocator{ nullptr };
		StackAllocator* transientAllocator{ nullptr };

		GTSL::Application systemApplication;

		Clock* clockInstance{ nullptr };
		InputManager* inputManagerInstance{ nullptr };
		ResourceManager* resourceManagerInstance{ nullptr };

		EventManager eventManager;

		UpdateContext updateContext{ UpdateContext::NORMAL };

		bool flaggedForClose = false;
		CloseMode closeMode{ CloseMode::OK };
		BE_DEBUG_ONLY(GTSL::String closeReason);

		uint64 applicationTicks{ 0 };
	private:
		inline static Application* applicationInstance{ nullptr };

	};

	Application* CreateApplication(GTSL::AllocatorReference* allocatorReference);
	void DestroyApplication(Application* application, GTSL::AllocatorReference* allocatorReference);
}
