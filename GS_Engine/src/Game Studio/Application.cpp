#include "Application.h"

#include "Logger.h"

#include "windows.h"

Clock * GS::Application::ClockInstance;
ResourceManager * GS::Application::ResourceManagerInstance;

namespace GS
{
	Application::Application()
	{
		GS_LOG_SUCCESS("Started Game Studio Engine!")

		ClockInstance = new Clock();
		WindowInstance = new Window(1280, 720, "Game Studio");
		RendererInstance = new Renderer(WindowInstance);
		EventDispatcherInstance = new EventDispatcher();
		InputManagerInstance = new InputManager();
	}

	Application::~Application()
	{
		delete ClockInstance;
		delete WindowInstance;
		delete RendererInstance;
		delete EventDispatcherInstance;
		delete InputManagerInstance;
	}

	void Application::Run()
	{
		while (true/*!ShouldClose()*/)
		{
			ClockInstance->OnUpdate();
			RendererInstance->OnUpdate();
			WindowInstance->OnUpdate();

			//Sleep(100);
		}	
	}

	/*int Application::ShouldClose()
	{
		//return WindowInstance;
	}*/
}