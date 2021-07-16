#pragma once

#include <GTSL/String.hpp>
#include <GTSL/Time.h>

class FunctionTimer
{
public:
	FunctionTimer(const GTSL::StaticString<64>& name);
	~FunctionTimer();
	
	GTSL::Microseconds StartingTime;
	GTSL::StaticString<64> Name;
};

#ifdef BE_DEBUG
//Places a timer which automatically starts counting. Timer will stop and print results when it exits the scope it was created in.
#define PROFILE() FunctionTimer profiler(u8 ##__FUNCTION__)
#else
#define PROFILE
#endif
