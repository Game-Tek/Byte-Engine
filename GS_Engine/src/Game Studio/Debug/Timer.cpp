#include "Timer.h"

#include "Application/Application.h"
#include "Logger.h"

Timer::Timer(const FString& _Name) : StartingTime(GS::Application::Get()->GetClock().GetCurrentNanoseconds()), Name(_Name)
{
}

Timer::~Timer()
{
    const Nanoseconds TimeTaken = GS::Application::Get()->GetClock().GetCurrentNanoseconds() - StartingTime;

    GS_BASIC_LOG_MESSAGE("Timer: %s, took %u nanoseconds.", Name.c_str(), TimeTaken)
}
