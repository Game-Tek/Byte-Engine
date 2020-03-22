#pragma once

#include "Application/Clock.h"

class Timer
{
	TimePoint startingTime;
	const char* name = "unnamed";
public:
	Timer(const char* name);
	~Timer();
};

#ifdef BE_DEBUG
//Places a timer which automatically starts counting. Timer will stop and print results when it exits the scope it was created in.
#define PLACE_TIMER(name) Timer LocalTimer(name);
#else
#define PLACE_TIMER(name)
#endif
