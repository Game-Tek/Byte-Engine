#pragma once

extern GS::Application* GS::CreateApplication();	//Is defined in another translation unit.

int main(int argc, char ** argv)
{
	auto Application = GS::CreateApplication();				//When CreateApplication() is defined it must return a new object of it class, effectively letting us manage that instance from here.
	const auto exit_code = Application->Run(argc, argv);	//Call Run() on Application. There lies the actual application code, like the Engine SubSystems' initialization, the game loop, etc.
	delete Application;										//When Run() is done we delete the instance.

	return exit_code;										//Return success and exit.
}