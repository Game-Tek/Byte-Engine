#include "ByteEngine/Application/Application.h"


#include <GTSL/Buffer.h>
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

		if (!parseConfig()) { Close(CloseMode::ERROR, GTSL::StaticString<64>("Failed to parse config file")); }
		
		BE_DEBUG_ONLY(closeReason = GTSL::String(255, systemAllocatorReference));
	}

	void Application::Shutdown()
	{
		if (closeMode != CloseMode::OK)
		{
			BE_LOG_WARNING("Shutting down application!\nReason: ", closeReason.c_str())
		}

		delete threadPool;

		settings.Free();
		
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

	bool Application::parseConfig()
	{
		GTSL::File settingsFile;

		settingsFile.OpenFile(GetPathToApplication() += "/settings.ini", GTSL::File::AccessMode::READ);

		//don't try parsing if file is empty or it's impractically large
		if(settingsFile.GetFileSize() == 0 || settingsFile.GetFileSize() > GTSL::Byte(GTSL::KiloByte(512))) { return false; }
		
		GTSL::SmartBuffer<PAR> fileBuffer(1024, 8, GetPersistentAllocator());

		settingsFile.ReadFile(fileBuffer);
		
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
		
		while (i < fileBuffer->GetLength())
		{			
			switch (static_cast<UTF8>(fileBuffer->GetData()[i]))
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
						settings.Emplace(key, value);
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
					text += static_cast<UTF8>(fileBuffer->GetData()[i]);
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
				if(text.IsEmpty())
				{
					auto value = processNumber();
					settings.Emplace(key, value);
					parseEnded = true;
					break;
				}

				parseEnded = false;
				break;
			}
			
		case Token::VALUE: break;
		}

		settingsFile.CloseFile();

		return parseEnded;
	}
}
