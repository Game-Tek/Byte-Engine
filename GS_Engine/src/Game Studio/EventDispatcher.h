#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include <functional>

#include "Event.h"

#include "Functor.h"

#include "FVector.hpp"

GS_CLASS EventDispatcher : public ESystem
{
public:
	EventDispatcher();
	~EventDispatcher();

	void OnUpdate() override;

	uint16 CreateEvent();
	void Subscribe(unsigned short EventId, Object * Subscriber, MemberFunctionPointer Func);
	void UnSubscribe(unsigned short EventId, Object * Subscriber);


private:
	//Determines which levels receive the events. Every level from the specified level upwards will get the events.
	uint8							ActiveLevel;

	uint16							EventCount;
	FVector<FVector<Functor>>		SubscriberInfo;
	FVector<Event *>				EventQueue;
public:
	template<typename T>
	void Notify(unsigned short EventId, const T & Event)
	{
		T * ptr = new T(Event);

		EventQueue.push_back(ptr);

		return;
	}
};