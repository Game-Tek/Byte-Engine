#include "Application.h"

#include "Logger.h"

#include "Windows.h"

Clock * GS::Application::ClockInstance;
ResourceManager * GS::Application::ResourceManagerInstance;
EventDispatcher * GS::Application::EventDispatcherInstance;
InputManager * GS::Application::InputManagerInstance;
GameInstance * GS::Application::GameInstanceInstance;



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
		ResourceManagerInstance = new ResourceManager();
		GameInstanceInstance = new GameInstance();
	}

	Application::~Application()
	{
		delete ClockInstance;
		delete WindowInstance;
		delete RendererInstance;
		delete EventDispatcherInstance;
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