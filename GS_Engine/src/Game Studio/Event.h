#pragma once

#include "Core.h"

GS_CLASS Event
{
public:
	Event(unsigned short EventId);
	~Event();

	unsigned short EventId;
};

