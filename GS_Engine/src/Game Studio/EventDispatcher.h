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

	static unsigned short CreateEvent();
	static void Subscribe(unsigned short EventId, Object * Subscriber, MemberFuncPtr Func);
	static void UnSubscribe(unsigned short EventId, Object * Subscriber);


private:
	//Determines which levels receive the events. Every level from the specified level upwards will get the events.
	static uint8					ActiveLevel;

	static unsigned short							EventCount;
	static FVector<FVector<Functor>>			SubscriberInfo;
	static FVector<Event *>							EventQueue;

	//void Dispatch(unsigned short Index);
	//static int Loop(unsigned short EventId);
public:
	template<typename T>
	static void Notify(unsigned short EventId, const T & Event)
	{
		T * ptr = new T(Event);

		EventQueue.push_back(ptr);

		return;
	}
};