#include "World.h"
#include "Application/Application.h"


World::World() : WorldObjects(10)
{
}



World::~World()
{
	for (auto& WOBJECT : WorldObjects)
	{
		delete WOBJECT;
	}
}

void World::OnUpdate()
{
	levelRunningTime += GS::Application::Get()->GetClock().GetDeltaTime();
	levelAdjustedRunningTime += GS::Application::Get()->GetClock().GetDeltaTime() * worldTimeMultiplier;

	for (auto& WorldObject : WorldObjects)
	{
		WorldObject->OnUpdate();
	}

	WorldScene.OnUpdate();
}

double World::GetRealRunningTime() { return GS::Application::Get()->GetClock().GetElapsedTime(); }

float World::GetWorldDeltaTime() const { return GS::Application::Get()->GetClock().GetDeltaTime() * worldTimeMultiplier; }
