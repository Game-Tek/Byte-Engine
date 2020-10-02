#include "ByteEngine/Application/Application.h"


#include <GTSL/Buffer.h>
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
		::new(&transientAllocator) StackAllocator(&systemAllocatorReference, 2, 2, 2048 * 2048 * 3);

		GTSL::Thread::SetThreadId(0);
		
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

		settings.Initialize(64, GetPersistentAllocator());
		
		BE_DEBUG_ONLY(closeReason = GTSL::String(255, systemAllocatorReference));

		{
			GTSL::File settingsFile;
			
			settingsFile.OpenFile(GetPathToApplication() += "/settings.ini", GTSL::File::AccessMode::READ);

			GTSL::Buffer fileBuffer; fileBuffer.Allocate(1024, 8, GetPersistentAllocator());

			settingsFile.ReadFile(fileBuffer);

			uint32 i = 0;
			
			while(static_cast<UTF8>(fileBuffer.GetData()[i]) != static_cast<UTF8>(-1) && i < fileBuffer.GetLength())
			{
				if(fileBuffer.GetData()[i] == '[')
				{
					while (fileBuffer.GetData()[i] != ']') { ++i; }
					i += 3;
				}

				GTSL::StaticString<128> key;
				
				while(fileBuffer.GetData()[i] != '=')
				{
					key += fileBuffer.GetData()[i];
					++i;
				}

				++i;
				
				GTSL::StaticString<128> valueString;

				while (fileBuffer.GetData()[i] != '\r' && static_cast<char>(fileBuffer.GetData()[i]) != static_cast<char>(-1) && i < fileBuffer.GetLength())
				{
					valueString += fileBuffer.GetData()[i];
					++i;
				}

				uint32 value = 0; uint32 mult = 1;
				
				for (uint32 j = 0, c = valueString.GetLength() - 2; j < valueString.GetLength() - 1; ++j, --c)
				{
					uint8 num;

					switch (valueString[c])
					{
					case '0': num = 0; break;
					case '1': num = 1; break;
					case '2': num = 2; break;
					case '3': num = 3; break;
					case '4': num = 4; break;
					case '5': num = 5; break;
					case '6': num = 6; break;
					case '7': num = 7; break;
					case '8': num = 8; break;
					case '9': num = 9; break;
					default: num = 0;
					}
					
					value += num * mult;

					mult *= 10;
				}
				
				//parse value

				settings.Emplace(Id(key.begin()), value);
			}
			
			fileBuffer.Free(8, GetPersistentAllocator());
			
			settingsFile.CloseFile();
		}
	}

	void Application::Shutdown()
	{
		if (closeMode != CloseMode::OK)
		{
			BE_LOG_WARNING("Shutting down application!\nReason: ", closeReason.c_str())
		}

		delete threadPool;
		
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

	uint8 Application::GetNumberOfThreads() { return threadPool->GetNumberOfThreads() + 1/*main thread*/; }

	void Application::OnUpdate(const OnUpdateInfo& updateInfo)
	{
		PROFILE;
		
		switch(updateInfo.UpdateContext)
		{
		case UpdateContext::NORMAL:
		{
			inputManagerInstance->Update();
			gameInstance->OnUpdate(this);
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

	void Application::Close(const CloseMode closeMode, const GTSL::Range<const UTF8*> reason)
	{
		closeReason.Append(reason);
		flaggedForClose = true;
		this->closeMode = closeMode;
	}
}