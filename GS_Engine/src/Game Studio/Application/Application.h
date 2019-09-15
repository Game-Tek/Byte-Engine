#pragma once

#include "../Core.h"

#include "Clock.h"
#include "InputManager.h"
#include "RAPI/Window.h"

namespace GS
{
	GS_CLASS Application : public Object
	{
		Clock ClockInstance;
		InputManager InputManagerInstance;

		Window* ActiveWindow = nullptr;

		static Application* ApplicationInstance;

		bool FlaggedForClose = false;

		[[nodiscard]] bool ShouldClose() const;

	public:
		Application();
		virtual ~Application();

		void Run();

		[[nodiscard]] const char* GetName() const override { return "Application"; }

		static Application * Get() { return ApplicationInstance; }

		//Updates the window the application gets it's context information from.
		void SetActiveWindow(Window* _NewWindow);

		//Fires a delegate to signal that the application has been requested to close.
		void PromptClose();

		//Flags the application to close on the next update.
		void Close() { FlaggedForClose = true; }

		[[nodiscard]] const Clock& GetClock() const { return ClockInstance; }
		[[nodiscard]] const InputManager& GetInputManager() const { return InputManagerInstance; }
		[[nodiscard]] const Window* GetActiveWindow() const { return ActiveWindow; }
	};

	Application * CreateApplication();
}
