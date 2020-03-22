#include "WorldObject.h"

#include "Application/Application.h"

#include "World.h"

World* WorldObject::GetWorld()
{
	return BE::Application::Get()->GetActiveWorld();
}
