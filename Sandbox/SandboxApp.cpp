#include "ByteEngine.h"

#include "Byte Engine/Application/Templates/GameApplication.h"

#include <iostream>

#include <GTSL/Window.h>

#include "Windows.h"

class Sandbox final : public GameApplication
{	
public:
	Sandbox() : GameApplication("Sandbox")
	{
	}

	void OnNormalUpdate() override
	{
		GameApplication::OnNormalUpdate();
	}

	void OnBackgroundUpdate() override
	{
	}
	
	~Sandbox()
	{
	}

	[[nodiscard]] const char* GetName() const override { return "Sandbox"; }
	const char* GetApplicationName() override { return "Sandbox"; }
};

BE::Application	* BE::CreateApplication()
{
	return new Sandbox();
}

void BE::DestroyApplication(Application* application)
{
	delete application;
}
