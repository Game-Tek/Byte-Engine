#pragma once

#include "Core.h"

#include "Containers/FString.h"
#include "Application/Clock.h"

class GS_API Timer
{
	Nanoseconds StartingTime = 0;
	FString Name = FString("No name!");
public:
	Timer(const FString& _Name);
	~Timer();
};

#ifdef GS_DEBUG
//Places a timer which automatically starts counting. Timer will stop and print results when it exits the scope it was created in.
#define PLACE_TIMER(name) Timer LocalTimer(FString(name));
#else
#define PLACE_TIMER(name)
#endif
