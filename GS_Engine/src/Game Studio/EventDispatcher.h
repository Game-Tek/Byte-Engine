#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include <functional>

#include "Event.h"

#include "Functor.h"

#include <vector>



GS_CLASS EventDispatcher : public ESystem
{
public:
	EventDispatcher();
	~EventDispatcher();

	void OnUpdate() override;

	static unsigned short CreateEvent();
	static void Subscribe(unsigned short EventId, Object * Subscriber, MemberFuncPtr Func);
	static void UnSubscribe(unsigned short EventId, Object * Subscriber);
	static void Notify(unsigned short Index, Event & Event);

private:
	//Determines which levels receive the events. Every level from the specified level upwards will get the events.
	static unsigned char					ActiveLevel;

	//static SArray<unsigned short>			Events;
	static unsigned short								EventCount;
	static std::vector<std::vector<Functor>>			SubscriberInfo;
	static std::vector<Event *>							EventQueue;

	void Dispatch(unsigned short Index);
	static int Loop(unsigned short EventId);
};