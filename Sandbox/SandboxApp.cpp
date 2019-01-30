#include <GameStudio.h>

#include "Game Studio/World.h"

#include "Game Studio/StaticMesh.h"

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{
		W.SpawnObject<StaticMesh>(StaticMesh("W:/lantern_fbx.fbx"));
	}

	~Sandbox()
	{

	}
private:
	World W;
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}