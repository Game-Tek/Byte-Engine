#pragma once

#include "Application.h"

extern int CreateApplication(GTSL::Range<const GTSL::StringView*> arguments); //Is defined in another translation unit.
//extern int CreateApplication(); //Is defined in another translation unit.

inline bool BasicCompatibilityTest() {
	return sizeof(utf8) == 1 && sizeof(uint8) == 1 && sizeof(uint16) == 2 && sizeof(uint32) == 4 && sizeof(uint64) == 8;
}

int main(int argc, char** argv)
{
	int exitCode = 0;
	
	if (!BasicCompatibilityTest()) { exitCode = -1; return exitCode; }
	
	GTSL::StringView arguments[32];

	for (uint8 i = 0; i < argc; ++i) {
		arguments[i] = GTSL::StringView((const utf8*)argv[i]);
	}

	// When CreateApplication() is defined it must return a new object of it class, effectively letting us manage that instance from here.
	exitCode = CreateApplication({ argc, arguments });

	return exitCode;
}

inline int do_default_flow(BE::Application* application) {
	int exitCode = -1;
	
	if (application->base_initialize({})) //call BE::Application initialize, which does basic universal startup
	{
		if (application->initialize()) //call BE::Application virtual initialize which will call the chain of initialize's
		{
			//Call Run() on Application. There lies the actual application code, like the Engine SubSystems' initialization, the game loop, etc.
			application->run();
		}
	}

	application->shutdown();

	return exitCode; //Return and exit.
}