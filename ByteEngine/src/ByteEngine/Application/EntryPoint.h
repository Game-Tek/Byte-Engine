#pragma once

#include "Application.h"

extern int CreateApplication(); //Is defined in another translation unit.

inline bool BasicCompatibilityTest() {
	return sizeof(utf8) == 1 && sizeof(uint8) == 1 && sizeof(uint16) == 2 && sizeof(uint32) == 4 && sizeof(uint64) == 8;
}

int main(int argc, char** argv)
{
	int exitCode = 0;
	
	if (!BasicCompatibilityTest()) { exitCode = -1; return exitCode; }
	
	//When CreateApplication() is defined it must return a new object of it class, effectively letting us manage that instance from here.
	exitCode = CreateApplication();

	return exitCode;
}

inline int Start(BE::Application* application) {
	int exitCode = 0;
	
	if (application->BaseInitialize(0, nullptr)) //call BE::Application initialize, which does basic universal startup
	{
		if (application->Initialize()) //call BE::Application virtual initialize which will call the chain of initialize's
		{
			application->PostInitialize();
			//Call Run() on Application. There lies the actual application code, like the Engine SubSystems' initialization, the game loop, etc.
			exitCode = application->Run(0, nullptr);
		}
	}

	application->Shutdown();

	return exitCode; //Return and exit.
}