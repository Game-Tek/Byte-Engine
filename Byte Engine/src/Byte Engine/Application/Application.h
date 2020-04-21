#pragma once

#include "Clock.h"
#include "InputManager.h"
#include "Byte Engine/Resources/ResourceManager.h"
#include "Byte Engine/Game/World.h"
#include "PoolAllocator.h"
#include "StackAllocator.h"
#include "SystemAllocator.h"
#include <GTSL/Application.h>

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
		friend struct TransientAllocatorReference;
	public:
		enum class CloseMode : uint8
		{
			OK, ERROR
		};

		struct TransientAllocatorReference : public GTSL::AllocatorReference
		{
			void Allocate(uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const override
			{
				Get()->transientAllocator->Allocate(size, alignment, memory, allocatedSize, "Transient");
			}

			void Deallocate(uint64 size, uint64 alignment, void* memory) const override
			{
				Get()->transientAllocator->Deallocate(size, alignment, memory, "Transient");
			}
		};
	private:
		inline static Application* applicationInstance{ nullptr };

	protected:
		struct SystemAllocatorReference : public GTSL::AllocatorReference
		{
			void Allocate(const uint64 size, uint64 alignment, void** memory, uint64* allocatedSize) const override
			{
				(*allocatedSize) = size;
				Get()->systemAllocator->Allocate(size, alignment, memory);
			}

			void Deallocate(const uint64 size, uint64 alignment, void* memory) const override
			{
				Get()->systemAllocator->Deallocate(size, alignment, memory);
			}
			
		} systemAllocatorReference;
		
		SystemAllocator* systemAllocator{ nullptr };
		PoolAllocator* poolAllocator{ nullptr };
		StackAllocator* transientAllocator{nullptr};

		TransientAllocatorReference transientAllocatorReference;

		GTSL::Application systemApplication;
		
		Clock* clockInstance{ nullptr };
		InputManager* inputManagerInstance{ nullptr };
		ResourceManager* resourceManagerInstance{ nullptr };
		
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
		
		[[nodiscard]] PoolAllocator* GetNormalAllocator() const { return poolAllocator; }
		[[nodiscard]] StackAllocator* GetTransientAllocator() const { return transientAllocator; }
	};

	Application* CreateApplication();
	void DestroyApplication(Application* application);
}
