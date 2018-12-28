#include "EventDispatcher.h"



EventDispatcher::EventDispatcher()
{
}


EventDispatcher::~EventDispatcher()
{
	for (unsigned short i = 0; i < Events.GetArrayLength(); i++)	//Delete every instanced event Events points to.
	{
		delete Events[i];
	}
}

void EventDispatcher::CreateEvent(unsigned short EventId)
{
	Event * NewEvent = new Event(EventId);
	Events.SetElement(NewEvent);		//Register event.

	return;
}

void EventDispatcher::Subscribe(void * Subscriber, unsigned short EventId, void(*FunctionToCall)())
{
	unsigned short Index;

	for (unsigned short i = 0; i < EventInfo.GetArrayLength(); i++)
	{
		if (Events[i]->EventId == EventId)
		{
			Index = i;

			break;
		}
	}
	EventInfo[Index][EventInfo.GetArrayLength()] = FunctionToCall;
}

void EventDispatcher::Post(unsigned short Index, Event & Event)
{
	EventInfo[0][0];
}

void EventDispatcher::Dispatch(unsigned short EventIndex)
{
	for (unsigned short i = 0; i < EventInfo[EventIndex].GetArrayLength(); i++)
	{
		EventInfo[EventIndex][i]();
	}
}
