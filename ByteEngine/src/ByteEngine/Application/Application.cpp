#include "ByteEngine/Application/Application.h"


#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/StaticString.hpp>

#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Clock.h"

#include "ByteEngine/Resources/ResourceManager.h"

#include "ByteEngine/Game/GameInstance.h"

#include "ByteEngine/Debug/FunctionTimer.h"
#include "ByteEngine/Debug/Logger.h"

#include <GTSL/System.h>
#include <GTSL/DataSizes.h>
#include <GTSL/Math/Math.hpp>

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

	bool Application::BaseInitialize(int argc, utf8* argv[])
	{
		if (!checkPlatformSupport()) {
			Close(CloseMode::ERROR, GTSL::StaticString<128>("No platform support."));
			return false;
		}

		::new(&poolAllocator) PoolAllocator(&systemAllocatorReference);
		::new(&transientAllocator) StackAllocator(&systemAllocatorReference, 2, 2, 2048 * 2048 * 3);

		GTSL::Thread::SetThreadId(0);

		resourceManagers.Initialize(8, systemAllocatorReference);

		systemApplication.SetProcessPriority(GTSL::Application::Priority::HIGH);

		Logger::LoggerCreateInfo logger_create_info;
		auto path = systemApplication.GetPathToExecutable();
		path.Drop(path.FindLast('/').Get());
		logger_create_info.AbsolutePathToLogDirectory = path;
		logger = GTSL::SmartPointer<Logger, BE::SystemAllocatorReference>::Create<Logger>(systemAllocatorReference, logger_create_info);

		inputManagerInstance = GTSL::SmartPointer<InputManager, BE::SystemAllocatorReference>::Create<InputManager>(systemAllocatorReference);
		threadPool = GTSL::SmartPointer<ThreadPool, BE::SystemAllocatorReference>::Create<ThreadPool>(systemAllocatorReference);

		settings.Initialize(64, GetPersistentAllocator());

		if (!parseConfig())	{
			Close(CloseMode::ERROR, GTSL::StaticString<64>("Failed to parse config file"));
		}

		initialized = true;

		BE_LOG_SUCCESS("Succesfully initialized Byte Engine module!");
		
		if (argc)
		{	
			GTSL::StaticString<2048> string("Application started with parameters:\n");

			for (uint32 p = 0; p < argc; ++p) {
				string += '	'; string += argv[p];
			}

			BE_LOG_MESSAGE(string);
		}
		else
		{
			BE_LOG_MESSAGE("Application started with no parameters.");
		}

		return true;
	}

	bool Application::Initialize()
	{
		return true;
	}

	void Application::Shutdown()
	{
		if (initialized)
		{
			gameInstance.TryFree();
			
			threadPool.TryFree(); //must free manually or else these smart pointers get freed on destruction, which is after the allocators (which this classes depend on) are destroyed.
			inputManagerInstance.TryFree();
			
			if (closeMode != CloseMode::OK)
			{
				if (closeMode == CloseMode::WARNING)
				{
					BE_LOG_WARNING("Shutting down application!\nReason: ", closeReason)
				}

				BE_LOG_ERROR("Shutting down application!\nReason: ", closeReason)
			}
			else
			{
				BE_LOG_SUCCESS("Shutting down application. No reported errors.")
			}

			settings.Free();
			resourceManagers.Free();
			
			transientAllocator.LockedClear();
			transientAllocator.Free();
			StackAllocator::DebugData stack_allocator_debug_data(&systemAllocatorReference);
			transientAllocator.GetDebugData(stack_allocator_debug_data);
			BE_LOG_MESSAGE("Debug data: ", static_cast<GTSL::StaticString<1024>>(stack_allocator_debug_data));

			logger.TryFree();
			
			poolAllocator.Free();
		}
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
		gameInstance->AddEvent("Application", EventHandle<>("OnPromptClose"));
		
		while (!flaggedForClose)
		{
			systemApplication.Update();
			
			clockInstance.OnUpdate();
			
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
		gameInstance->DispatchEvent("Application", EventHandle<>("OnPromptClose"));
	}

	void Application::Close(const CloseMode closeMode, const GTSL::Range<const utf8*> reason)
	{
		closeReason += reason;
		flaggedForClose = true;
		this->closeMode = closeMode;
	}

	bool Application::parseConfig()
	{
		GTSL::File settingsFile; settingsFile.OpenFile(GetPathToApplication() += "/settings.ini", GTSL::File::AccessMode::READ);

		//don't try parsing if file is empty
		if(settingsFile.GetFileSize() == 0) { return false; }
		
		GTSL::Buffer<TAR> fileBuffer; fileBuffer.Allocate(GTSL::Math::Limit(settingsFile.GetFileSize(), GTSL::Byte(GTSL::KiloByte(128)).GetCount()), 8, Object::GetTransientAllocator());

		settingsFile.ReadFile(fileBuffer.GetBufferInterface());
		
		uint32 i = 0;

		enum class Token
		{
			NONE, SECTION, KEY, VALUE
		} lastParsedToken = Token::NONE, currentToken = Token::NONE;

		GTSL::StaticString<128> text;
		bool parseEnded = false;
		Id key;
		
		while (i < fileBuffer.GetLength())
		{			
			switch (static_cast<utf8>(fileBuffer.GetData()[i]))
			{
			case '[':
				{
					if (lastParsedToken == Token::KEY) { return false; }
					currentToken = Token::SECTION;
					parseEnded = false;
					break;
				}

			case ']':
				{
					if (currentToken != Token::SECTION || lastParsedToken == Token::KEY) { return false; }
					parseEnded = !text.IsEmpty() && !parseEnded;
					if (!parseEnded) { return false; }

					key = text.begin();

					text.Resize(0);

					lastParsedToken = Token::SECTION;
					currentToken = Token::NONE;
					
					break;
				}

			case ' ':
				{
					return false;
				}

			case '=':
				{
					switch (lastParsedToken)
					{
					case Token::VALUE:
					case Token::SECTION:
					{
						if(currentToken != Token::NONE) { return false; }
						if (text.IsEmpty()) { return false; }
						key = text.begin();
						parseEnded = true;
						lastParsedToken = Token::KEY;
						currentToken = Token::VALUE;
						break;
					}
					case Token::KEY:
					case Token::NONE:
					{
						return false;
					}
					default: break;
					}

					text.Resize(0);
					break;
				}

			case '\0':
			case '\n':
			case '\r':
				{
					switch (lastParsedToken)
					{
					case Token::SECTION:
					{
						break;
					}
						
					case Token::KEY:
					{
						if(currentToken != Token::VALUE) { return false; }
						if (text.IsEmpty()) { return false; }
						auto value = GTSL::ToNumber<uint32>(text);
						if (!value.State()) { return false; }
						settings.Emplace(key, value.Get());
						lastParsedToken = Token::VALUE;
						currentToken = Token::NONE;
						parseEnded = true;
						break;
					}
						
					case Token::VALUE:
					{
						break;
					}
					case Token::NONE:
					{
						return false;
					}
						
					default: break;
					}

					text.Resize(0);
					break;
				}
				
			default:
				{
					if (text.GetLength() == 128) { return false; }
					text += static_cast<utf8>(fileBuffer.GetData()[i]);
				}
			}

			++i;
		}

		switch (lastParsedToken)
		{
		case Token::NONE:
			{
				parseEnded = false;
				break;
			}
			
		case Token::SECTION:
			{
				parseEnded = true;
				break;
			}
			
		case Token::KEY:
			{
				if(!text.IsEmpty())
				{
					auto value = GTSL::ToNumber<uint32>(text);
					if (!value.State()) { return false; }
					settings.Emplace(key, value.Get());
					parseEnded = true;
					break;
				}

				parseEnded = false;
				break;
			}
			
		case Token::VALUE: break;
		}

		return parseEnded;
	}
	
	bool Application::checkPlatformSupport()
	{
		bool sizeUTF8 = sizeof(utf8) == 1, size8 = sizeof(uint8) == 1, size16 = sizeof(uint16) == 2, size32 = sizeof(uint32) == 4, size64 = sizeof(uint64) == 8;

		GTSL::SystemInfo systemInfo;
		GTSL::System::GetSystemInfo(systemInfo);

		bool avx2 = systemInfo.CPU.VectorInfo.HW_AVX2;
		bool totalMemory = systemInfo.RAM.TotalPhysicalMemory >= GTSL::Byte(GTSL::GigaByte(12));
		bool availableMemory = systemInfo.RAM.ProcessAvailableMemory >= GTSL::Byte(GTSL::GigaByte(4));
		
		return sizeUTF8 && size8 && size16 && size32 && size64 && avx2 && totalMemory && availableMemory;
	}
}
