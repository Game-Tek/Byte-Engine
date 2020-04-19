#pragma once

#include "Application.h"

extern BE::Application* BE::CreateApplication(); //Is defined in another translation unit.
extern void BE::DestroyApplication(BE::Application* application); //Is defined in another translation unit.

int main(int argc, char** argv)
{
	SystemAllocator system_allocator;
	
	//When CreateApplication() is defined it must return a new object of it class, effectively letting us manage that instance from here.
	auto application = BE::CreateApplication();

	application->SetSystemAllocator(&system_allocator);

	application->Init();
	//Call Run() on Application. There lies the actual application code, like the Engine SubSystems' initialization, the game loop, etc.
	const auto exit_code = application->Run(argc, argv);
	
	BE::DestroyApplication(application);

	return exit_code; //Return and exit.
}
