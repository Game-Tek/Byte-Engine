#include "ByteEngine/Application/Application.h"

#include <GTSL/FlatHashMap.h>
#include <GTSL/StaticString.hpp>

#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Clock.h"

#include "ByteEngine/Resources/ResourceManager.h"

#include "ByteEngine/Game/GameInstance.h"

#include "ByteEngine/Debug/FunctionTimer.h"
#include "ByteEngine/Debug/Logger.h"

#if (_DEBUG)
void onAssert(const bool condition, const char* text, int line, const char* file, const char* function)
{
	//BE_BASIC_LOG_ERROR("GTSL ASSERT: ", text, ' ', "Line: ", line, ' ', "File: ", file, ' ', "Function: ", function);
}
#endif

namespace BE
{
	Application::Application(const ApplicationCreateInfo& ACI) : Object(ACI.ApplicationName), systemAllocatorReference("Application"),
	systemApplication(GTSL::Application::ApplicationCreateInfo{})
	{
		applicationInstance = this;
	}

	Application::~Application()
	{
	}

	void Application::Initialize()
	{
		::new(&poolAllocator) PoolAllocator(&systemAllocatorReference);
		::new(&transientAllocator) StackAllocator(&systemAllocatorReference, 2, 2, 2048 * 2048 * 2);
		
		resourceManagers.Initialize(8, systemAllocatorReference);
		
		systemApplication.SetProcessPriority(GTSL::Application::Priority::HIGH);

		Logger::LoggerCreateInfo logger_create_info;
		auto path = systemApplication.GetPathToExecutable();
		path.Drop(path.FindLast('/'));
		logger_create_info.AbsolutePathToLogDirectory = path;
		logger = GTSL::SmartPointer<Logger, BE::SystemAllocatorReference>::Create<Logger>(systemAllocatorReference, logger_create_info);
		
		clockInstance = new Clock();
		inputManagerInstance = new InputManager();
		threadPool = new ThreadPool();
		
		BE_DEBUG_ONLY(closeReason = GTSL::String(255, systemAllocatorReference));
	}

	void Application::Shutdown()
	{
		if (closeMode != CloseMode::OK)
		{
			BE_LOG_WARNING("Shutting down application!\nReason: ", closeReason.c_str())
		}
		
		delete clockInstance;
		delete inputManagerInstance;

		gameInstance.Free();
		
		transientAllocator.LockedClear();
		transientAllocator.Free();
		StackAllocator::DebugData stack_allocator_debug_data(&systemAllocatorReference);
		transientAllocator.GetDebugData(stack_allocator_debug_data);
		BE_LOG_MESSAGE("Debug data: ", static_cast<GTSL::StaticString<1024>>(stack_allocator_debug_data));

		poolAllocator.Free();
		
		logger->Shutdown();
	}

	void Application::OnUpdate(const OnUpdateInfo& updateInfo)
	{
		PROFILE;
		
		switch(updateInfo.UpdateContext)
		{
		case UpdateContext::NORMAL:
		{
			inputManagerInstance->Update();
		} break;
		
		case UpdateContext::BACKGROUND:
		{
			
		} break;
			
		default: break;
		}
	}

	int Application::Run(int argc, char** argv)
	{		
		while (!flaggedForClose)
		{
			systemApplication.Update();
			
			clockInstance->OnUpdate();
			
			OnUpdateInfo update_info{};
			update_info.UpdateContext = updateContext;
			OnUpdate(update_info);
			
			transientAllocator.LockedClear();

			++applicationTicks;
		}

		return static_cast<int>(closeMode);
	}

	void Application::PromptClose()
	{
		//CloseDelegate.Dispatch();
	}

	void Application::Close(const CloseMode closeMode, const GTSL::Ranger<const UTF8>& reason)
	{
		closeReason.Append(reason);
		flaggedForClose = true;
		this->closeMode = closeMode;
	}
}