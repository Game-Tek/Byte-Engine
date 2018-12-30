#pragma once

#include "Core.h"

#include "EngineSystem.h"

#include "Event.h"

#include "SArray.hpp"

typedef void (*FunctionPointer)();

GS_CLASS EventDispatcher : ESystem
{
public:
	EventDispatcher();
	~EventDispatcher();

	void CreateEvent(unsigned short EventId);
	void Subscribe(void * Subscriber, unsigned short EventId, void (* FunctionToCall)());
	void Post(unsigned short Index, Event & Event);

private:
	SArray<Event *>				Events;
	SArray<SArray<void (*)()>>	EventInfo;
	SArray<Event *>				EventQueue;

	void Dispatch(unsigned short Index);
};