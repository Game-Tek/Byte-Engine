#pragma once

#include "../Core.h"

#include "../Clock.h"
#include "../InputManager.h"

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

	private:
		Clock * ClockInstance = nullptr;
		InputManager * InputManagerInstance = nullptr;

		static Application * ApplicationInstance;

		/*int ShouldClose();*/
	};

	Application * CreateApplication();
}
