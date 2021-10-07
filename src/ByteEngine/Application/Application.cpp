#include "ByteEngine/Application/Application.h"


#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/String.hpp>

#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Clock.h"

#include "ByteEngine/Game/ApplicationManager.h"

#include "ByteEngine/Debug/Logger.h"

#include <GTSL/System.h>
#include <GTSL/Math/Math.hpp>

#if (_DEBUG)
void onAssert(const bool condition, const char* text, int line, const char* file, const char* function)
{
	//BE_BASIC_LOG_ERROR("GTSL ASSERT: ", text, ' ', "Line: ", line, ' ', "File: ", file, ' ', "Function: ", function);
}
#endif

namespace BE
{
	Application::Application(GTSL::ShortString<128> applicationName) : Object(applicationName.begin()), systemAllocator(), systemAllocatorReference(this, applicationName.begin()), resourceManagers(8, systemAllocatorReference),
	settings(32, systemAllocatorReference), systemApplication(GTSL::Application::ApplicationCreateInfo{}),
	transientAllocator(&systemAllocatorReference, 2, 2, 2048 * 2048 * 4)
	{
		applicationInstance = this;
	}

	Application::~Application()
	{
	}

	bool Application::BaseInitialize(int argc, utf8* argv[])
	{
		if (!checkPlatformSupport()) {
			Close(CloseMode::ERROR, GTSL::StaticString<128>(u8"No platform support."));
			return false;
		}

		::new(&poolAllocator) PoolAllocator(&systemAllocatorReference);

		GTSL::Thread::SetThreadId(0);

		systemApplication.SetProcessPriority(GTSL::Application::Priority::HIGH);

		Logger::LoggerCreateInfo logger_create_info;
		auto path = systemApplication.GetPathToExecutable();
		DropLast(path, u8'/');
		logger_create_info.AbsolutePathToLogDirectory = path;
		logger = GTSL::SmartPointer<Logger, SystemAllocatorReference>(systemAllocatorReference, logger_create_info);

		inputManagerInstance = GTSL::SmartPointer<InputManager, SystemAllocatorReference>(systemAllocatorReference);

		if (!parseConfig())	{
			Close(CloseMode::ERROR, GTSL::StaticString<64>(u8"Failed to parse config file"));
			return false;
		}

		{
			auto threadCount = GetOption(u8"threadCount");
			threadCount = GTSL::Math::Limit(threadCount, static_cast<uint32>(GTSL::Thread::ThreadCount() - 1/*main thread*/));
			threadCount = threadCount ? static_cast<uint8>(threadCount) : GTSL::Thread::ThreadCount();
			threadPool = GTSL::SmartPointer<ThreadPool, SystemAllocatorReference>(systemAllocatorReference, threadCount);
		}
		
		initialized = true;

		BE_LOG_SUCCESS(u8"Succesfully initialized Byte Engine module!");
		
		if (argc > 0) {	
			GTSL::StaticString<2048> string(u8"Application started with parameters:\n");

			for (uint32 p = 0; p < argc; ++p) {
				string += '	'; string += argv[p];
			}

			BE_LOG_MESSAGE(string);
		} else {
			BE_LOG_MESSAGE(u8"Application started with no parameters.");
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
					BE_LOG_WARNING(u8"Shutting down application!\nReason: ", closeReason)
				}

				BE_LOG_ERROR(u8"Shutting down application!\nReason: ", closeReason)
			}
			else
			{
				BE_LOG_SUCCESS(u8"Shutting down application. No reported errors.")
			}

			settings.Free();
			resourceManagers.Free();
			
			transientAllocator.LockedClear();
			transientAllocator.Free();
			StackAllocator::DebugData stack_allocator_debug_data(&systemAllocatorReference);
			transientAllocator.GetDebugData(stack_allocator_debug_data);
			BE_LOG_MESSAGE(u8"Debug data: ", static_cast<GTSL::StaticString<1024>>(stack_allocator_debug_data));

			logger.TryFree();
			
			poolAllocator.Free();
		}
	}

	uint8 Application::GetNumberOfThreads() { return threadPool->GetNumberOfThreads() + 1/*main thread*/; }

	void Application::OnUpdate(const OnUpdateInfo& updateInfo)
	{		
		inputManagerInstance->Update();
		gameInstance->OnUpdate(this);
	}

	int Application::Run(int argc, char** argv)
	{
		gameInstance->AddEvent(u8"Application", EventHandle(u8"OnPromptClose"));
		
		while (!flaggedForClose) {			
			clockInstance.OnUpdate();
			
			OnUpdateInfo update_info{};
			OnUpdate(update_info);
			
			transientAllocator.LockedClear();

			++applicationTicks;
		}

		return static_cast<int>(closeMode);
	}

	void Application::PromptClose()
	{
		gameInstance->DispatchEvent(u8"Application", EventHandle(u8"OnPromptClose"));
	}

	void Application::Close(const CloseMode closeMode, const GTSL::Range<const utf8*> reason)
	{
		closeReason += reason;
		flaggedForClose = true;
		this->closeMode = closeMode;
	}

	bool Application::parseConfig()
	{
		auto path = GetPathToApplication();
		path += u8"/settings.ini";
		
		GTSL::File settingsFile; settingsFile.Open(path, GTSL::File::READ, false);

		//don't try parsing if file is empty
		if(settingsFile.GetSize() == 0) { return false; }
		
		GTSL::Buffer fileBuffer(GTSL::Math::Limit(settingsFile.GetSize(), GTSL::Byte(GTSL::KiloByte(128)).GetCount()), 8, Object::GetTransientAllocator());

		settingsFile.Read(fileBuffer);
		
		uint32 i = 0;

		enum class Token
		{
			NONE, SECTION, KEY, VALUE
		} lastParsedToken = Token::NONE, currentToken = Token::NONE;

		GTSL::StaticString<128> text;
		bool parseEnded = false;
		Id key;
		
		while (i < fileBuffer.GetLength()) {			
			switch (static_cast<utf8>(fileBuffer.GetData()[i])) {
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

					key = text.c_str();

					text.Drop(0);

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
						key = text.c_str();
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

					text.Drop(0);
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

					text.Drop(0);
					break;
				}
				
			default:
				{
					if (text.GetBytes() == 128) { return false; }
					text += static_cast<utf8>(fileBuffer.GetData()[i]);
				}
			}

			++i;
		}

		switch (lastParsedToken) {
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
	
	bool Application::checkPlatformSupport() {
		GTSL::SystemInfo systemInfo;
		GTSL::System::GetSystemInfo(systemInfo);

		bool avx2 = systemInfo.CPU.VectorInfo.HW_AVX2;
		bool totalMemory = systemInfo.RAM.TotalPhysicalMemory >= GTSL::Byte(GTSL::GigaByte(12));
		bool availableMemory = systemInfo.RAM.ProcessAvailableMemory >= GTSL::Byte(GTSL::GigaByte(2));
		
		return avx2 && totalMemory && availableMemory;
	}
}
