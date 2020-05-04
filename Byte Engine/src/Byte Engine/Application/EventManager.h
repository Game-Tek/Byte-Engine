#pragma once

#include <unordered_map>
#include <GTSL/Id.h>

class EventManager
{
	struct EventTypeBase
	{
		virtual void AddEvent();
	};
	
	template<typename T>
	struct EventType : EventTypeBase
	{
		GTSL::Vector<T> events;
		
		void AddEvent() override
		{
			
		}
	};
	
	std::unordered_map<GTSL::Id64::HashType, EventTypeBase*> events;
public:
	template<typename T>
	void AddEvent(const GTSL::Id64& name)
	{
		events.insert({ name, new EventType<T>() });
	}
	
	void SubscribeToEvent(const GTSL::Id64& name)
	{
		events.at(name)->AddEvent();
	}
	
	void UnsubscribeToEvent(const GTSL::Id64& name)
	{
		events.at(name);
	}
};
