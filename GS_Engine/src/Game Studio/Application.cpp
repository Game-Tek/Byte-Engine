#include "Application.h"

#include "Logger.h"

#include "windows.h"

namespace GS
{
	Application::Application()
	{
		GS_LOG_SUCCESS("Started Game Studio Engine!")

		ClockInstance = new Clock();
		WindowInstance = new Window(1280, 720, "Game Studio");
		RendererInstance = new Renderer(WindowInstance);
		//EventDispatcherInstance = new EventDispatcher();
	}

	Application::~Application()
	{
		delete ClockInstance;
		delete WindowInstance;
		delete RendererInstance;
		//delete EventDispatcherInstance;
	}

	void Application::Run()
	{
		while (true/*!ShouldClose()*/)
		{
			ClockInstance->OnUpdate();
			RendererInstance->OnUpdate(ClockInstance->GetDeltaTime());

			Sleep(1000);
		}	
	}

	/*int Application::ShouldClose()
	{
		//return WindowInstance;
	}*/
}