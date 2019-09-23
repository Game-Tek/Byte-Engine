#include "World.h"

World::~World()
{
	for (auto& WOBJECT : WorldObjects)
	{
		DestroyWorldObject(WOBJECT);
	}
}