#include "Application.h"

#include <GTSL/StaticString.hpp>

#include "ByteEngine/Resources/AudioResourceManager.h"
#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Debug/FunctionTimer.h"

#if (_DEBUG)
void onAssert(const bool condition, const char* text, int line, const char* file, const char* function)
{
	BE_BASIC_LOG_ERROR("GTSL ASSERT: ", text, ' ', "Line: ", line, ' ', "File: ", file, ' ', "Function: ", function);
}
#endif

namespace BE
{
	Application::Application(const ApplicationCreateInfo& ACI) : systemAllocatorReference("Application"),
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
		::new(&transientAllocator) StackAllocator(&systemAllocatorReference, 2, 2, 2048 * 2048);
		
		systemApplication.SetProcessPriority(GTSL::Application::Priority::HIGH);

		Logger::LoggerCreateInfo logger_create_info;
		auto path = systemApplication.GetPathToExecutable();
		path.Drop(path.FindLast('/'));
		logger_create_info.AbsolutePathToLogDirectory = path;
		GTSL::Allocation<Logger>::Create<Logger>(systemAllocatorReference, logger, logger_create_info);
		
		clockInstance = new Clock();
		inputManagerInstance = new InputManager();
		
		BE_DEBUG_ONLY(closeReason = GTSL::String(255, GetPersistentAllocator()));
	}

	void Application::Shutdown()
	{
		if (closeMode != CloseMode::OK)
		{
			BE_LOG_WARNING("Shutting down application!\nReason: ", closeReason.c_str())
		}

		closeReason.Free(GetPersistentAllocator());
		
		delete clockInstance;
		delete inputManagerInstance;

		transientAllocator.LockedClear();
		transientAllocator.Free();
		StackAllocator::DebugData stack_allocator_debug_data(&systemAllocatorReference);
		transientAllocator.GetDebugData(stack_allocator_debug_data);
		BE_LOG_MESSAGE("Debug data: ", static_cast<GTSL::StaticString<1024>>(stack_allocator_debug_data));

		poolAllocator.Free();
		
		logger->Shutdown();
		GTSL::Delete(logger, systemAllocatorReference);
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
		closeReason.Append(reason, GetPersistentAllocator());
		flaggedForClose = true;
		this->closeMode = closeMode;
	}
}