#pragma once

#include "Clock.h"
#include "InputManager.h"
#include "Byte Engine/Resources/ResourceManager.h"
#include "Byte Engine/Game/World.h"
#include "PoolAllocator.h"
#include "StackAllocator.h"
#include "SystemAllocator.h"
#include <GTSL/Application.h>
#include <GTSL/Allocator.h>

#include "EventManager.h"

struct SystemAllocatorReference : public GTSL::AllocatorReference
{
protected:
	void allocateFunc(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

	void deallocateFunc(uint64 size, uint64 alignment, void* memory) const;

public:
	SystemAllocatorReference() : AllocatorReference(GTSL::FunctionPointer<void(uint64, uint64, void**, uint64*)>::Create<SystemAllocatorReference, &SystemAllocatorReference::allocateFunc>(), GTSL::FunctionPointer<void(uint64, uint64, void*)>::Create<SystemAllocatorReference, &SystemAllocatorReference::deallocateFunc>())
	{
	}

};

struct TransientAllocatorReference : public GTSL::AllocatorReference
{
protected:
	void allocateFunc(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const;

	void deallocateFunc(uint64 size, uint64 alignment, void* memory) const;

public:
	TransientAllocatorReference() : AllocatorReference(GTSL::FunctionPointer<void(uint64, uint64, void**, uint64*)>::Create<TransientAllocatorReference, &TransientAllocatorReference::allocateFunc>(), GTSL::FunctionPointer<void(uint64, uint64, void*)>::Create<TransientAllocatorReference, &TransientAllocatorReference::deallocateFunc>())
	{

	}
};

namespace BE
{
	/**
	 * \brief Defines all the data necessary to startup a GameStudio application instance.
	 */
	struct ApplicationCreateInfo
	{
		const char* ApplicationName = nullptr;
	};

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
		SystemAllocatorReference systemAllocatorReference;
		TransientAllocatorReference transientAllocatorReference;
		
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
		
		[[nodiscard]] SystemAllocator* GetSystemAllocator() const { return systemAllocator; }
		[[nodiscard]] PoolAllocator* GetNormalAllocator() const { return poolAllocator; }
		[[nodiscard]] StackAllocator* GetTransientAllocator() const { return transientAllocator; }
	};

	Application* CreateApplication(GTSL::AllocatorReference* allocatorReference);
	void DestroyApplication(Application* application, GTSL::AllocatorReference* allocatorReference);
}