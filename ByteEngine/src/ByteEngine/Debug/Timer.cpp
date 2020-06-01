#include "Timer.h"

#include "Logger.h"
#include "ByteEngine/Application/Application.h"

Timer::Timer(const char* name) : startingTime(BE::Application::Get()->GetClock()->GetCurrentTime()), name(name)
{
}

Timer::~Timer()
{
	const auto time_taken = BE::Application::Get()->GetClock()->GetCurrentTime() - startingTime;

	BE_BASIC_LOG_MESSAGE("Timer: ", name, "took ", time_taken.GetCount(), " milliseconds")
}
