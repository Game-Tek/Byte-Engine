#include "World.h"


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
