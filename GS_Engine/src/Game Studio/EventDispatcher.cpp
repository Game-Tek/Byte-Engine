#include "EventDispatcher.h"

unsigned char					EventDispatcher::ActiveLevel = 0;

//SArray<unsigned short>			EventDispatcher::Events;
unsigned short					EventDispatcher::EventCount = 0;
SArray<SArray<FunctionPointer>>	EventDispatcher::EventInfo;
SArray<const Event &>			EventDispatcher::EventQueue;

EventDispatcher::EventDispatcher()
{
}

EventDispatcher::~EventDispatcher()
{
}

void EventDispatcher::OnUpdate()
{
	for (unsigned short i = 0; i < EventQueue.GetArrayLength(); i++)
	{
		for (unsigned short j = 0; j < EventInfo[EventQueue[i].EventIndex].GetArrayLength(); j++)
		{
			EventInfo[EventQueue[i].EventIndex][j](EventQueue[i]);
		}
		EventQueue.RemoveElement(i);
	}
}

unsigned short EventDispatcher::CreateEvent()
{
	//Events.PopBack(EventId);

	EventCount++;

	return EventCount;
}

void EventDispatcher::Subscribe(unsigned short EventId, FunctionPointer FunctionToCall)
{
	//unsigned short EventIndex = Loop(EventId);		//Call loop and store the return in _local_var_EventIndex.

	EventInfo[EventId].PopBack(FunctionToCall);			//Access EventInfo at _local_var_EventIndex and store in the array inside that index the function to call.

	return;
}

void EventDispatcher::UnSubscribe(unsigned short EventId, FunctionPointer OrigFunction)
{
	/*for (unsigned short i = 0; i < EventInfo.GetArrayLength(); i++)
	{
		for (unsigned short j = 0; j < EventInfo[i].GetArrayLength(); j++)
		{
			if (EventInfo[i][j] == OrigFunction);
			{
				EventInfo[i].RemoveElement(j, false);
			}
		}
	}
	*/

	for (unsigned short i = 0; i < EventInfo[EventId].GetArrayLength(); i++)
	{
		if (EventInfo[EventId][i] == OrigFunction)
		{
			EventInfo[EventId].RemoveElement(i);

			break;
		}
	}

	return;
}

void EventDispatcher::Notify(unsigned short EventId, Event & Event)
{
	EventQueue.PopBack(Event);

	//Event.EventIndex = Loop(EventId);

	return;
}

/*
//Find index for EventId.
int EventDispatcher::Loop(unsigned short EventId)
{
	for (unsigned short i = 0; i < Events.GetArrayLength(); i++)	//Loop through each registered event.
	{
		if (Events[i] = EventId)									//If _param_EventId equals the EventId in the current index return i.
		{
			return i;
		}
	}
	return;
}
*/