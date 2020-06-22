#pragma once

#include <GTSL/Time.h>

class FunctionTimer
{
public:
	FunctionTimer(const char* name);
	~FunctionTimer();
	
	GTSL::Microseconds StartingTime;
	const char* Name = "unnamed";
};

#ifdef BE_DEBUG
//Places a timer which automatically starts counting. Timer will stop and print results when it exits the scope it was created in.
#define PROFILE FunctionTimer profiler(__FUNCTION__)
#else
#define PROFILE
#endif
