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

		static const ResourceManager * GetResourceManager() { return ResourceManagerInstance; }
		static const Clock * GetClock() { return ClockInstance; }

	private:
		static Clock * ClockInstance;

		Window * WindowInstance = nullptr;

		Renderer * RendererInstance = nullptr;

		EventDispatcher * EventDispatcherInstance = nullptr;

		InputManager * InputManagerInstance = nullptr;

		static ResourceManager * ResourceManagerInstance;

		/*int ShouldClose();*/
	};

	Application * CreateApplication();
}