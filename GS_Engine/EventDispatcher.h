#pragma once

#include "Core.h"

#include "Event.h"

GS_CLASS EventDispatcher
{
public:
	EventDispatcher();
	~EventDispatcher();

private:
	Event EventQueue[100];
};

