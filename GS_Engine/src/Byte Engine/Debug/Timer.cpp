#include "Timer.h"

#include "Application/Application.h"
#include "Logger.h"

Timer::Timer(const char* name) : startingTime(BE::Application::Get()->GetClock().GetCurrentTime()), name(name)
{
}

Timer::~Timer()
{
	const auto time_taken = BE::Application::Get()->GetClock().GetCurrentTime() - startingTime;

	BE_BASIC_LOG_MESSAGE("Timer: %s, took %lf milliseconds", name, time_taken.Milliseconds<double>())
}
