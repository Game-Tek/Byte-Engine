#include "ByteEngine/Application/Application.h"

#include "ByteEngine/Application/InputManager.h"
#include "ByteEngine/Application/ThreadPool.h"
#include "ByteEngine/Application/Clock.h"
#include "ByteEngine/Game/ApplicationManager.h"
#include "ByteEngine/Debug/Logger.h"

#include <filesystem>

#include <GTSL/Application.h>
#include <GTSL/Buffer.hpp>
#include <GTSL/DataSizes.h>
#include <GTSL/HashMap.hpp>
#include <GTSL/String.hpp>
#include <GTSL/Math/Math.hpp>
#include <GTSL/System.hpp>

#if (_DEBUG)
void onAssert(const bool condition, const char* text, int line, const char* file, const char* function)
{
	//BE_BASIC_LOG_ERROR("GTSL ASSERT: ", text, ' ', "Line: ", line, ' ', "File: ", file, ' ', "Function: ", function);
}
#endif

GTSL::uint32 ShitTracker::shitCount = 0;

namespace BE
{
	Application::Application(GTSL::StringView applicationName) : Object(applicationName) {
		applicationInstance = this;
	}

	Application::~Application() {}

	bool Application::base_initialize(GTSL::Range<const GTSL::StringView*> arguments) {
		if (!checkPlatformSupport() ) {
			Close(CloseMode::ERROR, GTSL::StaticString<128>(u8"No platform support."));
			return false;
		}

		GTSL::Thread::SetThreadId(0);

		poolAllocator.initialize();
		stackAllocator.initialize(BE::SystemAllocatorReference(u8"StackAllocator", false));

		//systemApplication.SetProcessPriority(GTSL::Application::Priority::HIGH);

		Logger::LoggerCreateInfo logger_create_info;
		auto path = GetPathToApplication();
		logger_create_info.LogDirAbsolutePath = path;
		logger = GTSL::SmartPointer<Logger, SystemAllocatorReference>(BE::SystemAllocatorReference(u8"Logger", true), logger_create_info);

		if (!parseConfig()) { // TODO. create config file if it doesn't exist.
			return false;
		}

		//logger->SetTrace(GetBoolOption(u8"trace"));

		// inputManagerInstance = GTSL::SmartPointer<InputManager, SystemAllocatorReference>(systemAllocatorReference);

#if BE_PLATFORM_WINDOWS
		// Set the process DPI awareness so windows' extents match the set resolution and are not scaled by Windows to match size based on DPI.
		// We are basically saying that we are aware of DPI and we are handling the scaling ourselves.
		SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
#endif

		{
			auto threadCount = 1u;
			//auto threadCount = (uint32)GetUINTOption(u8"threadCount");
			threadCount = GTSL::Math::Limit(threadCount, static_cast<GTSL::uint32>(GTSL::Thread::ThreadCount() - 1/*main thread*/));
			threadCount = threadCount ? static_cast<GTSL::uint8>(threadCount) : GTSL::Thread::ThreadCount();
			threadPool = GTSL::SmartPointer<ThreadPool, SystemAllocatorReference>(BE::SystemAllocatorReference(u8"ThreadPool"), threadCount);
		}
		
		initialized = true;

		BE_LOG_SUCCESS(u8"Succesfully initialized Byte Engine module!");
		
		if (arguments.ElementCount() > 0) {	
			GTSL::StaticString<2048> string(u8"Application started with parameters:\n\t");

			string += GTSL::Join{ arguments, u8" " };

			BE_LOG_MESSAGE(string);
		} else {
			BE_LOG_MESSAGE(u8"Application started with no parameters.");
		}

		return true;
	}

	bool Application::initialize() {
		applicationManager = GTSL::SmartPointer<ApplicationManager, BE::SystemAllocatorReference>(BE::SystemAllocatorReference(u8"ApplicationManager"));

		return true;
	}

	void Application::shutdown() {
		if(logger) { // If logger instance was successfully initialized report shutting down reason
			if (closeMode != CloseMode::OK) {
				if (closeMode == CloseMode::WARNING) {
					BE_LOG_WARNING(u8"Shutting down application!\nReason: ", closeReason)
				}

				BE_LOG_ERROR(u8"Shutting down application!\nReason: ", closeReason)
			}
			else {
				BE_LOG_SUCCESS(u8"Shutting down application. No reported errors.")
			}
		}

		if (initialized) {
			//must free manually or else these smart pointers get freed on destruction, which is after the allocators (which this classes depend on) are destroyed.

			applicationManager.TryFree();
			threadPool.TryFree();
			//inputManagerInstance.TryFree();
			logger.TryFree();			
			stackAllocator.clear();
			poolAllocator.free();
		}
	}

	//uint8 Application::GetNumberOfThreads() { return threadPool->GetNumberOfThreads() + 1/*main thread*/; }
	GTSL::uint8 Application::GetNumberOfThreads() { return 1/*main thread*/; }

	void Application::OnUpdate(const OnUpdateInfo& updateInfo) {
		// inputManagerInstance->Update();
		// applicationManager->OnUpdate(this);
	}

	void Application::run() {
		//applicationManager->AddEvent(u8"Application", EventHandle(u8"OnPromptClose"));
		
		while (!flaggedForClose) {			
			//clockInstance.OnUpdate();
			
			OnUpdateInfo update_info{};
			OnUpdate(update_info);
			
			//transientAllocator.LockedClear();

			++applicationTicks;
		}
	}

	void Application::PromptClose() {
		//applicationManager->DispatchEvent(this, EventHandle(u8"OnPromptClose"));
	}

	void Application::Close(const CloseMode closeMode, const GTSL::Range<const char8_t*> reason)
	{
		closeReason += reason;
		flaggedForClose = true;
		this->closeMode = closeMode;
	}

	GTSL::StaticString<260> Application::GetPathToApplication() const {
#ifdef _WIN32
		char s[260];
		GetModuleFileNameA(GetModuleHandleA(nullptr), s, 260);

		GTSL::StaticString<260> ret(reinterpret_cast<const char8_t*>(s));
		ReplaceAll(ret, u8'\\', u8'/');
		RTrimLast(ret, u8'/');
		return ret;
#else
		auto path = GTSL::Application::GetPathToExecutable();
		RTrimLast(path, u8'/');
		return path;
#endif
	}

	bool Application::parseConfig() {
		auto path = GetPathToApplication();
		path += u8"/settings.json";
		
		GTSL::File settingsFile;
		switch (settingsFile.Open(path, GTSL::File::READ, false)) {
		case GTSL::File::OpenResult::ERROR: Close(CloseMode::ERROR, GTSL::StaticString<64>(u8"Config file not found.")); return false;
		}

		//don't try parsing if file is empty
		if(settingsFile.GetSize() == 0) {
			Close(CloseMode::ERROR, GTSL::StaticString<64>(u8"Config file is empty."));
			return false;
		}
		
		GTSL::Buffer fileBuffer(settingsFile, Object::GetTransientAllocator());

		JSON = GTSL::MoveRef(GTSL::JSON(GTSL::StringView(fileBuffer), GTSL::DefaultAllocatorReference{}));

		return true;
	}
	
	bool Application::checkPlatformSupport() {
		GTSL::SystemInfo systemInfo = GTSL::System::GetSystemInfo();

		bool avx2 = systemInfo.CPU.VectorInfo.HW_AVX2;
		bool totalMemory = systemInfo.RAM.TotalPhysicalMemory.GetCount() >= GTSL::Byte(GTSL::GigaByte(12)).GetCount();
		//bool availableMemory = systemInfo.RAM.ProcessAvailableMemory >= GTSL::Byte(GTSL::GigaByte(2));
		bool availableMemory = true;
		
		return avx2 && totalMemory && availableMemory;
	}
}
