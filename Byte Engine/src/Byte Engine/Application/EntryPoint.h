#pragma once

#include "Application.h"

extern BE::Application* BE::CreateApplication(SystemAllocator* systemAllocator); //Is defined in another translation unit.
extern void BE::DestroyApplication(BE::Application* application, SystemAllocator* systemAllocator); //Is defined in another translation unit.

int main(int argc, char** argv)
{
	SystemAllocator system_allocator;
	
	//When CreateApplication() is defined it must return a new object of it class, effectively letting us manage that instance from here.
	auto application = BE::CreateApplication(&system_allocator);
	//Call Run() on Application. There lies the actual application code, like the Engine SubSystems' initialization, the game loop, etc.
	const auto exit_code = application->Run(argc, argv);
	
	BE::DestroyApplication(application, &system_allocator);

	return exit_code; //Return and exit.
}
