#pragma once

#include "Application.h"

extern BE::Application* BE::CreateApplication(); //Is defined in another translation unit.

int main(int argc, char** argv)
{
	//When CreateApplication() is defined it must return a new object of it class, effectively letting us manage that instance from here.
	auto Application = BE::CreateApplication();
	//Call Run() on Application. There lies the actual application code, like the Engine SubSystems' initialization, the game loop, etc.
	const auto exit_code = Application->Run(argc, argv);
	
	delete Application; //When Run() is done we delete the instance.

	return exit_code; //Return and exit.
}
