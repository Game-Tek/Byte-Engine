#include <GameStudio.h>

#include "Game Studio/World.h"

#include "Game Studio/StaticMesh.h"
#include "Character.h"

class Sandbox : public GS::Application
{
public:
	Sandbox()
	{
		Vector3 Vec(0, 0, -100.0f);
		GetGameInstanceInstance()->GetWorld()->SpawnObject(new StaticMesh(), Vec);
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