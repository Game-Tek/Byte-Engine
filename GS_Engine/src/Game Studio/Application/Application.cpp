#include "Application.h"

#include "Logger.h"

GS::Application * GS::Application::ApplicationInstance;

namespace GS
{
	Application::Application()
	{
		ApplicationInstance = this;

		ClockInstance = new Clock();
		InputManagerInstance = new InputManager();

		WindowCreateInfo WCI;
		WCI.Extent = { 1280, 720 };
		WCI.Name = "Game Studio!";
		WCI.WindowType = WindowFit::NORMAL;
		WindowInstance = Window::CreateGSWindow(WCI);
	}

	Application::~Application()
	{
		delete ClockInstance;
		delete InputManagerInstance;
		delete WindowInstance;
	}

	void Application::Run()
	{
		while (!ShouldClose())
		{
			ClockInstance->OnUpdate();
			WindowInstance->Update();

			//Sleep(100);
		}	
	}

	bool Application::ShouldClose()
	{
		return WindowInstance->GetShouldClose();
	}
}