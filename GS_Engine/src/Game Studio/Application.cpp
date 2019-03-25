#include "Application.h"

#include "Logger.h"

#include "Windows.h"

GS::Application * GS::Application::ApplicationInstance;

namespace GS
{
	Application::Application()
	{
		GS_LOG_SUCCESS("Started Game Studio Engine!")

		ApplicationInstance = this;

		ClockInstance = new Clock();
		WindowInstance = new Window(1280, 720, "Game Studio");
		RendererInstance = new Renderer(WindowInstance);
		InputManagerInstance = new InputManager();
		ResourceManagerInstance = new ResourceManager();
		GameInstanceInstance = new GameInstance();
	}

	Application::~Application()
	{
		delete ClockInstance;
		delete WindowInstance;
		delete RendererInstance;
		delete InputManagerInstance;
		delete ResourceManagerInstance;
		delete GameInstanceInstance;
	}

	void Application::Run()
	{
		while (true/*!ShouldClose()*/)
		{
			ClockInstance->OnUpdate();
			RendererInstance->OnUpdate();
			WindowInstance->OnUpdate();
			GameInstanceInstance->OnUpdate();

			//Sleep(100);
		}	
	}

	/*int Application::ShouldClose()
	{
		//return WindowInstance;
	}*/
}