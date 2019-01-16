#pragma once

#include "Core.h"

#include "Clock.h"

#include "Window.h"

#include "Renderer.h"

#include "EventDispatcher.h"

namespace GS
{
	GS_CLASS Application
	{
	public:
		Application();
		virtual ~Application();

		void Run();

	private:
		Clock * ClockInstance = nullptr;

		Window * WindowInstance = nullptr;

		Renderer * RendererInstance = nullptr;

		EventDispatcher * EventDispatcherInstance = nullptr;

		/*int ShouldClose();*/
	};

	Application * CreateApplication();
}