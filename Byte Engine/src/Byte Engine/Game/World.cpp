#include "World.h"
#include "Application/Application.h"

World::World()
{
}

World::~World()
{
	for(auto& e : types)
	{
		delete e.second;
	}
}

void World::OnUpdate()
{
	levelRunningTime += BE::Application::Get()->GetClock().GetDeltaTime();
	levelAdjustedRunningTime += BE::Application::Get()->GetClock().GetDeltaTime() * worldTimeMultiplier;

	for(auto& e : types)
	{
		TypeManager::UpdateInstancesInfo update_instances_info;
		e.second->UpdateInstances(update_instances_info);
	}
}

void World::Pause()
{
	worldTimeMultiplier = 0;
}

double World::GetRealRunningTime() { return BE::Application::Get()->GetClock().GetElapsedTime().Seconds<double>(); }

GTSL::TimePoint World::GetWorldDeltaTime() const { return BE::Application::Get()->GetClock().GetDeltaTime() * worldTimeMultiplier; }
