#pragma once

#include <iostream>

#include "Byte Engine/Application/Application.h"

#include <GTSL/Window.h>

class GameApplication : public BE::Application
{
	GTSL::Window window;
	
	void resize(const GTSL::Extent2D& size)
	{

	}
public:
	GameApplication(const char* name) : Application(BE::ApplicationCreateInfo{ name })
	{
	}

	void Init() override
	{
		Application::Init();
		
		GTSL::Window::WindowCreateInfo create_window_info;
		create_window_info.Application = &systemApplication;
		create_window_info.Name = GTSL::String(GetApplicationName(), &transientAllocatorReference);
		create_window_info.Extent = { 1280, 720 };
		::new(&window) GTSL::Window(create_window_info);
		
		//window.SetOnResizeDelegate(Delegate<void(const GTSL::Extent2D&)>::Create<GameApplication, &GameApplication::resize>(this));

		//window.ShowWindow();
	}
	
	void OnNormalUpdate() override
	{
		std::cout << "Game Application loop update\n";
		//systemApplication.UpdateWindow(&window);
	}
};
