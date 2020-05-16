#pragma once

#include "World.h"

inline void* operator new(const uint64 size, GTSL::AllocatorReference* allocatorReference)
{
	void* alloc{ nullptr }; uint64 alloc_size{ 0 }; allocatorReference->Allocate(size, 8, &alloc, &alloc_size); return alloc;
}

class GameInstance
{
	GTSL::Vector<World*> worlds;

public:
	using WorldReference = uint8;
	
	struct CreateNewWorldInfo
	{
		class BE::Application* Application{ nullptr };
	};
	template<typename T>
	WorldReference CreateNewWorld(const CreateNewWorldInfo& createNewWorldInfo)
	{
		//worlds.PushBack(new(createNewWorldInfo.Application) T());
		return 0;
	}

	template<typename T>
	void UnloadWorld(const WorldReference worldId)
	{
		World::DestroyInfo destroy_info;
		worlds[worldId]->DestroyWorld(destroy_info);
		worlds.Destroy(worldId);
	}
};
