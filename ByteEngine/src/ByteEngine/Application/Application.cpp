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

	bool Application::BaseInitialize(int argc, UTF8* argv[])
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
		path.Drop(path.FindLast('/'));
		logger_create_info.AbsolutePathToLogDirectory = path;
		logger = GTSL::SmartPointer<Logger, BE::SystemAllocatorReference>::Create<Logger>(systemAllocatorReference, logger_create_info);

		inputManagerInstance = GTSL::SmartPointer<InputManager, BE::SystemAllocatorReference>::Create<InputManager>(systemAllocatorReference);
		threadPool = GTSL::SmartPointer<ThreadPool, BE::SystemAllocatorReference>::Create<ThreadPool>(systemAllocatorReference);

		settings.Initialize(64, GetPersistentAllocator());

		if (!parseConfig()) { Close(CloseMode::ERROR, GTSL::StaticString<64>("Failed to parse config file")); }

		initialized = true;

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
	}

	bool Application::Initialize()
	{
		return true;
	}

	void Application::Shutdown()
	{
		if (initialized)
		{
			gameInstance.Free();
			
			threadPool.Free(); //must free manually or else these smart pointers get freed on destruction, which is after the allocators (which this classes depend on) are destroyed.
			inputManagerInstance.Free();
			
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

			logger.Free();
			
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
		//CloseDelegate.Dispatch();
	}

	void Application::Close(const CloseMode closeMode, const GTSL::Range<const UTF8*> reason)
	{
		closeReason += reason;
		flaggedForClose = true;
		this->closeMode = closeMode;
	}

	bool Application::parseConfig()
	{
		GTSL::File settingsFile;

		settingsFile.OpenFile(GetPathToApplication() += "/settings.ini", GTSL::File::AccessMode::READ);

		//don't try parsing if file is empty or it's impractically large
		if(settingsFile.GetFileSize() == 0 || settingsFile.GetFileSize() > GTSL::Byte(GTSL::KiloByte(512))) { return false; }
		
		GTSL::Buffer<PAR> fileBuffer; fileBuffer.Allocate(1024, 8, GetPersistentAllocator());

		settingsFile.ReadFile(fileBuffer.GetBufferInterface());
		
		uint32 i = 0;

		enum class Token
		{
			NONE, SECTION, KEY, VALUE
		} lastParsedToken = Token::NONE, currentToken = Token::NONE;

		GTSL::StaticString<128> text;
		bool parseEnded = false;
		Id key;

		auto processNumber = [&]() -> uint32
		{
			uint32 value = 0, mult = 1;

			for (uint32 j = 0, c = text.GetLength() - 2/*one for null terminator, another for index vs length*/; j < text.GetLength() - 1; ++j, --c)
			{
				uint8 num;

				switch (text[c])
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

			return value;
		};
		
		while (i < fileBuffer.GetLength())
		{			
			switch (static_cast<UTF8>(fileBuffer.GetData()[i]))
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
						auto value = processNumber();
						settings.Emplace(key(), value);
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
					text += static_cast<UTF8>(fileBuffer.GetData()[i]);
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
					auto value = processNumber();
					settings.Emplace(key(), value);
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
		bool size_8 = sizeof(uint8) == 1; bool size_16 = sizeof(uint16) == 2; bool size_32 = sizeof(uint32) == 4; bool size_64 = sizeof(uint64) == 8;

		GTSL::SystemInfo systemInfo;
		GTSL::System::GetSystemInfo(systemInfo);

		bool avx2 = systemInfo.CPU.VectorInfo.HW_AVX2;
		bool memory = systemInfo.RAM.ProcessAvailableMemory >= GTSL::Byte(GTSL::GigaByte(6));
		
		return size_8 && size_16 && size_32 && size_64 && avx2;
	}
}
