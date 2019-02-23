#include "WorldObject.h"
#include "Application.h"

WorldObject::WorldObject(const Transform3 & Transform) : Transform(Transform)
{
}

GameInstance* WorldObject::GetGameInstance()
{
	return GS::Application::Get()->GetGameInstanceInstance();
}
