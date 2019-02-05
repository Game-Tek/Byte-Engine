#include <GameStudio.h>

#include "Game Studio/World.h"

#include "Game Studio/StaticMesh.h"

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{
		StaticMesh SM (std::string("W:/lantern_fbx.fbx"));
		Vector3 Vec(0, 0, 0);
		W.SpawnObject<StaticMesh>(SM, Vec);
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