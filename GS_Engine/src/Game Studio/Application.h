#pragma once

#include "Core.h"

#include "Clock.h"
#include "Window.h"
#include "Renderer.h"
#include "EventDispatcher.h"
#include "InputManager.h"
#include "ResourceManager.h"
#include "GameInstance.h"

namespace GS
{
	GS_CLASS Application
	{
	public:
		Application();
		virtual ~Application();

		void Run();

		//TO-DO: CHECK CONST FOR POINTERS.

		static Renderer * GetRendererInstance() { return RendererInstance; }
		static EventDispatcher * GetEventDispatcherInstance() { return EventDispatcherInstance; }
		static ResourceManager * GetResourceManagerInstance() { return ResourceManagerInstance; }
		static Clock * GetClockInstance() { return ClockInstance; }
		static InputManager * GetInputManagerInstance() { return InputManagerInstance; }
		static GameInstance * GetGameInstanceInstance() { return GameInstanceInstance; }

	private:
		static Clock * ClockInstance;

		Window * WindowInstance = nullptr;

		static Renderer * RendererInstance;

		static EventDispatcher * EventDispatcherInstance;

		static InputManager * InputManagerInstance;

		static ResourceManager * ResourceManagerInstance;

		static GameInstance * GameInstanceInstance;

		/*int ShouldClose();*/
	};

	Application * CreateApplication();
}