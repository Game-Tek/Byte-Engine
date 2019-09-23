#include <GameStudio.h>

#include <string>
#include <iostream>
#include <Game Studio/Debug/Logger.h>

//#include <Game Studio/Utility/FlipFlop.h>
#include "Game Studio/Game/World.h"
#include "TestObject.h"

class Framebuffer;

class Sandbox final : public GS::Application
{
public:
	Sandbox()
	{
		MyObject.SetPosition(Vector3(0, 0, 25));
		//auto D = Functor::MakeDelegate(&Window::GetAspectRatio, Win);
		//GS_BASIC_LOG_MESSAGE("%f", D());
	}

	void OnUpdate() final override
	{
		MyWorld.OnUpdate();
	}

	~Sandbox()
	{
	}

	World MyWorld;
	TestObject MyObject;
	//FlipFlop Flip;
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}