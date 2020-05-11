#pragma once

#include "Byte Engine/Core.h"

#include <GTSL/Application.h>
#include <GTSL/Allocator.h>
#include <GTSL/String.hpp>


#include "Clock.h"
#include "InputManager.h"

#include "Byte Engine/Resources/ResourceManager.h"

#include "PoolAllocator.h"
#include "StackAllocator.h"
#include "SystemAllocator.h"

#include "EventManager.h"
#include "Byte Engine/Debug/Logger.h"

namespace BE
{
	struct BEAllocatorReference : GTSL::AllocatorReference
	{
		const char* Name{ nullptr };
		bool IsDebugAllocation = false;

		explicit BEAllocatorReference(const decltype(allocate)& allocateFunc, const decltype(deallocate)& deallocateFunc, const char* name, const bool isDebugAllocation = false) : AllocatorReference(allocateFunc, deallocateFunc), Name(name), IsDebugAllocation(isDebugAllocation) {}
	};
	
	struct SystemAllocatorReference : BEAllocatorReference
	{
	protected:
		void allocateFunc(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void deallocateFunc(uint64 size, uint64 alignment, void* memory) const;

	public:
		SystemAllocatorReference(const char* name, const bool isDebugAllocation = false) :
			BEAllocatorReference(reinterpret_cast<decltype(allocate)>(&SystemAllocatorReference::allocateFunc), reinterpret_cast<decltype(deallocate)>(&SystemAllocatorReference::deallocateFunc),
			name, isDebugAllocation)
		{
		}

	};
	
	struct TransientAllocatorReference : BEAllocatorReference
	{
	protected:		
		void allocateFunc(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void deallocateFunc(uint64 size, uint64 alignment, void* memory) const;

	public:
		TransientAllocatorReference(const char* name, const bool isDebugAllocation = false) :
			BEAllocatorReference(reinterpret_cast<decltype(allocate)>(&TransientAllocatorReference::allocateFunc), reinterpret_cast<decltype(deallocate)>(&TransientAllocatorReference::deallocateFunc),
			name, isDebugAllocation)
		{
		}
	};

	struct PersistentAllocatorReference : BEAllocatorReference
	{
	protected:
		const char* name{ nullptr };
		
		void allocateFunc(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

		void deallocateFunc(uint64 size, uint64 alignment, void* memory) const;

	public:
		PersistentAllocatorReference(const char* name, const bool isDebugAllocation = false) :
			BEAllocatorReference(reinterpret_cast<decltype(allocate)>(&PersistentAllocatorReference::allocateFunc), reinterpret_cast<decltype(deallocate)>(&PersistentAllocatorReference::deallocateFunc),
			name, isDebugAllocation)
		{
		}
	};
	
	/**
	 * \brief Defines all the data necessary to startup a GameStudio application instance.
	 */
	struct ApplicationCreateInfo
	{
		const char* ApplicationName = nullptr;
	};

#undef ERROR
	
	class Application : public Object
	{
	public:
		enum class CloseMode : uint8
		{
			OK, ERROR
		};
	private:
		inline static Application* applicationInstance{ nullptr };

	protected:
		Logger* logger{ nullptr };
		
		SystemAllocatorReference systemAllocatorReference;
		
		SystemAllocator* systemAllocator{ nullptr };
		PoolAllocator* poolAllocator{ nullptr };
		StackAllocator* transientAllocator{ nullptr };

		GTSL::Application systemApplication;
		
		Clock* clockInstance{ nullptr };
		InputManager* inputManagerInstance{ nullptr };
		ResourceManager* resourceManagerInstance{ nullptr };

		EventManager eventManager;
		
		bool isInBackground = false;
		
		bool flaggedForClose = false;
		CloseMode closeMode{ CloseMode::OK };
		GTSL::String closeReason;

		[[nodiscard]] bool shouldClose() const;
	public:
		explicit Application(const ApplicationCreateInfo& ACI);
		virtual ~Application();

		void SetSystemAllocator(SystemAllocator* newSystemAllocator) { systemAllocator = newSystemAllocator; }

		virtual void Init() = 0;
		
		virtual void OnNormalUpdate() = 0;
		virtual void OnBackgroundUpdate() = 0;
		
		int Run(int argc, char** argv);
		
		virtual const char* GetApplicationName() = 0;
		[[nodiscard]] static const char* GetEngineName() { return "Byte Engine"; }
		static const char* GetEngineVersion() { return "0.0.1"; }

		static Application* Get() { return applicationInstance; }

		//Fires a Delegate to signal that the application has been requested to close.
		void PromptClose();

		//Flags the application to close on the next update.
		void Close(const CloseMode closeMode, const char* reason);

		[[nodiscard]] const Clock* GetClock() const { return clockInstance; }
		[[nodiscard]] const InputManager* GetInputManager() const { return inputManagerInstance; }
		[[nodiscard]] ResourceManager* GetResourceManager() const { return resourceManagerInstance; }
		[[nodiscard]] EventManager* GetEventManager() { return &eventManager; }
		[[nodiscard]] Logger* GetLogger() const { return logger; }
		
		[[nodiscard]] SystemAllocator* GetSystemAllocator() const { return systemAllocator; }
		[[nodiscard]] PoolAllocator* GetNormalAllocator() const { return poolAllocator; }
		[[nodiscard]] StackAllocator* GetTransientAllocator() const { return transientAllocator; }

#ifdef BE_DEBUG
#define BE_LOG_SUCCESS(Text, ...)		BE::Application::Get()->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::SUCCESS, Text, __VA_ARGS__);
#define BE_LOG_MESSAGE(Text, ...)		BE::Application::Get()->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::MESSAGE, Text, __VA_ARGS__);
#define BE_LOG_WARNING(Text, ...)		BE::Application::Get()->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::WARNING, Text, __VA_ARGS__);
#define BE_LOG_ERROR(Text, ...)			BE::Application::Get()->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::FATAL, Text, __VA_ARGS__);
#define BE_LOG_LEVEL(Level)				BE::Application::Get()->GetLogger()->SetMinLogLevel(Level);

#define BE_BASIC_LOG_SUCCESS(Text, ...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::SUCCESS, Text, __VA_ARGS__);
#define BE_BASIC_LOG_MESSAGE(Text, ...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::MESSAGE, Text, __VA_ARGS__);
#define BE_BASIC_LOG_WARNING(Text, ...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::WARNING, Text, __VA_ARGS__);
#define BE_BASIC_LOG_ERROR(Text, ...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::FATAL, Text, __VA_ARGS__);
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
	};

	Application* CreateApplication(GTSL::AllocatorReference* allocatorReference);
	void DestroyApplication(Application* application, GTSL::AllocatorReference* allocatorReference);
}
