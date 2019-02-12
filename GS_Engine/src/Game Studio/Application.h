#pragma once

#include "Core.h"

#include "Clock.h"
#include "Window.h"
#include "Renderer.h"
#include "EventDispatcher.h"
#include "InputManager.h"
#include "ResourceManager.h"

namespace GS
{
	GS_CLASS Application
	{
	public:
		Application();
		virtual ~Application();

		void Run();

		//TO-DO: CHECK CONST FOR POINTERS.

		static EventDispatcher * GetEventDispatcherInstance() { return EventDispatcherInstance; }
		static ResourceManager * GetResourceManagerInstance() { return ResourceManagerInstance; }
		static Clock * GetClockInstance() { return ClockInstance; }
		static InputManager * GetInputManagerInstance() { return InputManagerInstance; }

	private:
		static Clock * ClockInstance;

		Window * WindowInstance = nullptr;

		Renderer * RendererInstance = nullptr;

		static EventDispatcher * EventDispatcherInstance;

		static InputManager * InputManagerInstance;

		static ResourceManager * ResourceManagerInstance;

		/*int ShouldClose();*/
	};

	Application * CreateApplication();
}