#include "Timer.h"

#include "Application.h"
#include "Logger.h"

Timer::Timer(const FString& _Name) : StartingTime(Application::Get()->GetClock().GetCurrentNanoseconds()), Name(_Name)
{
}

Timer::~Timer()
{
    const Nanoseconds TimeTaken = Application::Get()->GetClock().GetCurrentNanoseconds() - StartingTime;

    GS_LOG_MESSAGE("Timer: %s, took %u nanoseconds.", Name.c_str(), TimeTaken)
}
