#include <GameStudio.h>

#include "Game Studio/World.h"

#include "Game Studio/StaticMesh.h"
#include "Game Studio/PointLight.h"
#include "Character.h"

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{
		GetGameInstanceInstance()->GetWorld()->SpawnObject(new StaticMesh(String("W:/Box.obj")), Vector3(0, 50, 0));
		GetGameInstanceInstance()->GetWorld()->SpawnObject(new StaticMesh(String("W:/Floor.obj")), Vector3());
		GetGameInstanceInstance()->GetWorld()->SpawnObject(new PointLight(), Vector3(0, 0, 0));
		GetGameInstanceInstance()->GetWorld()->SpawnObject(new Character(), Vector3());
	}

	~Sandbox()
	{

	}
};

GS::Application	* GS::CreateApplication()
{
	return new Sandbox();
}