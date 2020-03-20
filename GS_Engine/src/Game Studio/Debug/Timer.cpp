#include "Timer.h"

#include "Application/Application.h"
#include "Logger.h"

Timer::Timer(const char* name) : startingTime(GS::Application::Get()->GetClock().GetCurrentTime()), name(name)
{
}

Timer::~Timer()
{
	const auto time_taken = GS::Application::Get()->GetClock().GetCurrentTime() - startingTime;

	GS_BASIC_LOG_MESSAGE("Timer: %s, took %lf milliseconds", name, time_taken.Milliseconds<double>())
}
