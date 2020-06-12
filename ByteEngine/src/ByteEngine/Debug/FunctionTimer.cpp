#include "FunctionTimer.h"

#include "Logger.h"
#include "ByteEngine/Application/Application.h"

FunctionTimer::FunctionTimer(const char* name) : StartingTime(BE::Application::Get()->GetClock()->GetCurrentTime()), Name(name)
{
}

FunctionTimer::~FunctionTimer()
{
	BE::Application::Get()->GetLogger()->logFunctionTimer(this, BE::Application::Get()->GetClock()->GetCurrentTime() - StartingTime);
}
