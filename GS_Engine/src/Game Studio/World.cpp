#include "World.h"


World::World()
{
}


World::~World()
{
	for (uint32 i = 0; i < EntityList.size(); i++)
	{
		delete EntityList[i];
	}
}
