#include "FunctionTimer.h"

#include "Logger.h"
#include "ByteEngine/Application/Application.h"

#undef GetCurrentTime

FunctionTimer::FunctionTimer(const char* name) : StartingTime(BE::Application::Get()->GetClock()->GetCurrentMicroseconds()), Name(name)
{
}

FunctionTimer::~FunctionTimer()
{
	BE::Application::Get()->GetLogger()->logFunctionTimer(this, BE::Application::Get()->GetClock()->GetCurrentMicroseconds() - StartingTime);
}
