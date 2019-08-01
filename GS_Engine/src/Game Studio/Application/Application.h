#pragma once

#include "../Core.h"

#include "../Clock.h"
#include "../InputManager.h"
#include "RAPI/Window.h"

namespace GS
{
	GS_CLASS Application
	{
	public:
		Application();
		virtual ~Application();

		void Run();

		static Application * Get() { return ApplicationInstance; }

		//TO-DO: CHECK CONST FOR POINTERS.

		Clock * GetClockInstance() const { return ClockInstance; }
		InputManager * GetInputManagerInstance() const { return InputManagerInstance; }
		Window* GetWindow() const { return WindowInstance; }

	private:
		Clock * ClockInstance = nullptr;
		InputManager * InputManagerInstance = nullptr;
		Window* WindowInstance = nullptr;

		static Application * ApplicationInstance;

		bool ShouldClose();
	};

	Application * CreateApplication();
}
