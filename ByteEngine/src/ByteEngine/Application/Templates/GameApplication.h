#pragma once

#include "ByteEngine/Application/Application.h"

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

	void Init() override;

	void OnNormalUpdate() override;
};
