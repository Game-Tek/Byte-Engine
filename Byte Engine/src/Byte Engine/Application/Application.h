#pragma once

#include "Clock.h"
#include "InputManager.h"
#include "Byte Engine/Resources/ResourceManager.h"
#include "Byte Engine/Game/World.h"
#include "BigAllocator.h"
#include "StackAllocator.h"

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
		static Application* applicationInstance;

	protected:
		BigAllocator bigAllocator;
		StackAllocator transientAllocator;
		
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
		
		[[nodiscard]] BigAllocator* GetBigAllocator() { return &bigAllocator; }
		[[nodiscard]] StackAllocator* GetTransientAllocator() { return &transientAllocator; }
	};

	Application* CreateApplication();
}
