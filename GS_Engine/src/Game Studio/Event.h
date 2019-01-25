#pragma once

#include "Core.h"

GS_STRUCT Event
{
public:
	unsigned short EventId = 0;

	Event()
	{
	}

	Event(uint16 Id) : EventId(Id)
	{
	}

	~Event()
	{
	}
};

