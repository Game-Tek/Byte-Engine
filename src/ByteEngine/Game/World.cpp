#include "World.h"

#include "GTSL/JSON.hpp"

World::World()
{
}

void World::InitializeWorld(const InitializeInfo& initializeInfo)
{
	//GTSL::JSONMember json;
	//
	////name = json[u8"name"];
	//
	//for(auto e : json[u8"elements"]) {
	//	if(auto m = e[u8"Mesh"]) {
	//		m[u8"name"];
	//		auto pos = m[u8"pos"];
	//		GTSL::Vector3(pos[0].GetFloat(), pos[1].GetFloat(), pos[2].GetFloat());
	//	}
	//}
}

void World::DestroyWorld(const DestroyInfo& destroyInfo)
{
}

void World::Pause()
{
	worldTimeMultiplier = 0;
}
