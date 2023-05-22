#include "FunctionTimer.h"

#include "ByteEngine/Application/Application.h"

#undef GetCurrentTime

FunctionTimer::FunctionTimer(const GTSL::StaticString<64>& name)
	: StartingTime(BE::Application::Get()->GetClock()->GetCurrentMicroseconds()), Name(name)
{
}

FunctionTimer::~FunctionTimer()
{
	
}

