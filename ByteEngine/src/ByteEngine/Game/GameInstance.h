#pragma once

#include <GTSL/Delegate.hpp>

#include "World.h"

#include <GTSL/FlatHashMap.h>
#include <GTSL/Id.h>

class GameInstance : public Object
{
public:
	GameInstance();
	virtual ~GameInstance();

	[[nodiscard]] const char* GetName() const override { return "GameInstance"; }
	
	virtual void OnUpdate();
	
	using WorldReference = uint8;

	template<typename T>
	T* AddSystem(const GTSL::Id64 systemName)
	{
		schedulerSystems.Emplace(GetPersistentAllocator(), systemName);
		auto ret = static_cast<T*>(systems.Emplace(GetPersistentAllocator(), systemName, GTSL::Allocation<System>::Create<T>(GetPersistentAllocator()))->Data);
		initSystem(ret, systemName); return ret;
	}

	template<typename T>
	T* AddComponentCollection(const GTSL::Id64 componentCollectionName)
	{
		GTSL::Allocation<ComponentCollection> pointer = *componentCollections.Emplace(GetPersistentAllocator(), componentCollectionName, GTSL::Allocation<ComponentCollection>::Create<T>(GetPersistentAllocator()));
		initCollection(pointer); return static_cast<T*>(pointer.Data);
	}
	
	struct CreateNewWorldInfo
	{
	};
	template<typename T>
	WorldReference CreateNewWorld(const CreateNewWorldInfo& createNewWorldInfo)
	{
		auto index = worlds.EmplaceBack(GTSL::Allocation<World>::Create<T>(GetPersistentAllocator()));
		initWorld(index); return index;
	}

	template<typename T>
	void UnloadWorld(const WorldReference worldId)
	{
		World::DestroyInfo destroy_info;
		destroy_info.GameInstance = this;
		worlds[worldId]->DestroyWorld(destroy_info);
		GTSL::Delete(worlds[worldId], GetPersistentAllocator());
		worlds.Destroy(worldId);
	}

	class ComponentCollection* GetComponentCollection(const GTSL::Id64 collectionName) { return componentCollections.At(collectionName); }
	class ComponentCollection* GetComponentCollection(const GTSL::Id64 collectionName, GTSL::FlatHashMap<GTSL::Allocation<ComponentCollection>>::ElementReference& reference) { reference = componentCollections.GetReference(collectionName); return componentCollections.At(collectionName); }
	class ComponentCollection* GetComponentCollection(const GTSL::FlatHashMap<GTSL::Allocation<ComponentCollection>>::ElementReference collectionReference) { return componentCollections[collectionReference]; }

	class System* GetSystem(const GTSL::Id64 systemName) { return systems.At(systemName); }
	class System* GetSystem(const GTSL::Id64 systemName, GTSL::FlatHashMap<GTSL::Allocation<System>>::ElementReference& reference) { reference = systems.GetReference(systemName); return systems.At(systemName); }
	class System* GetSystem(const GTSL::FlatHashMap<GTSL::Allocation<System>>::ElementReference systemReference) { return systems[systemReference]; }

	struct TaskInfo
	{
	};
	
	enum class AccessType : uint8 { READ, READ_WRITE };
	void AddTask(GTSL::Id64 name, AccessType accessType, GTSL::Delegate<void(const TaskInfo&)> function, GTSL::Ranger<GTSL::Id64> actsOn, GTSL::Id64 doneFor);
	void AddGoal(GTSL::Id64 name, GTSL::Id64 dependsOn); void AddGoal(GTSL::Id64 name);
private:
	GTSL::Vector<GTSL::Allocation<World>> worlds;
	GTSL::FlatHashMap<GTSL::Allocation<System>> systems;
	GTSL::FlatHashMap<GTSL::Allocation<ComponentCollection>> componentCollections;

	struct SchedulerSystem
	{
		struct Goal
		{		
			Goal();

			void AddTask(const GTSL::Delegate<void(const TaskInfo&)> function);
			void AddNewTaskStack();

			GTSL::Vector<GTSL::Vector<GTSL::Delegate<void(const TaskInfo&)>>> ParallelTasks;
		};

		SchedulerSystem();
		
		GTSL::Vector<Goal> goals;
		bool nextNeedsNewStack = true;
	};
	GTSL::FlatHashMap<SchedulerSystem> schedulerSystems;
	GTSL::Vector<GTSL::Id64> goalNames;
	
	void initWorld(uint8 worldId);
	void initCollection(ComponentCollection* collection);
	void initSystem(System* system, GTSL::Id64 name);
};
