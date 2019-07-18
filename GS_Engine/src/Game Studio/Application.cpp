#include "Application.h"

#include "Logger.h"

GS::Application * GS::Application::ApplicationInstance;

namespace GS
{
	Application::Application()
	{
		GS_LOG_SUCCESS("Started Game Studio Engine!")

		ApplicationInstance = this;

		ClockInstance = new Clock();
		InputManagerInstance = new InputManager();
		GameInstanceInstance = new GameInstance();
	}

	Application::~Application()
	{
		delete ClockInstance;
		delete WindowInstance;
		delete RendererInstance;
		delete InputManagerInstance;
		delete GameInstanceInstance;
	}

	void Application::Run()
	{
		while (true/*!ShouldClose()*/)
		{
			ClockInstance->OnUpdate();
			GameInstanceInstance->OnUpdate();

			//Sleep(100);
		}	
	}

	/*int Application::ShouldClose()
	{
		//return WindowInstance;
	}*/
}