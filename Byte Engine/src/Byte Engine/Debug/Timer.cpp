#include "Timer.h"

#include "Logger.h"
#include "Byte Engine/Application/Application.h"

Timer::Timer(const char* name) : startingTime(BE::Application::Get()->GetClock()->GetCurrentTime()), name(name)
{
}

Timer::~Timer()
{
	const auto time_taken = BE::Application::Get()->GetClock()->GetCurrentTime() - startingTime;

	BE_BASIC_LOG_MESSAGE("Timer: %s, took %luu milliseconds", name, time_taken.GetCount())
}
