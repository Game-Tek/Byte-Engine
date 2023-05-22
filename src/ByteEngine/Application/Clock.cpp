#include "Clock.h"

#if BE_PLATFORM_WINDOWS
#include <Windows.h>
#endif

#include <chrono>

Clock::Clock()
{
#if BE_PLATFORM_WINDOWS
	LARGE_INTEGER WinProcessorFrequency;
	LARGE_INTEGER WinProcessorTicks;

	QueryPerformanceFrequency(&WinProcessorFrequency);
	QueryPerformanceCounter(&WinProcessorTicks);

	processorFrequency = WinProcessorFrequency.QuadPart;
	startPerformanceCounterTicks = WinProcessorTicks.QuadPart;
	performanceCounterTicks = WinProcessorTicks.QuadPart;
#endif
}

Clock::~Clock() = default;

void Clock::OnUpdate() {
#if BE_PLATFORM_WINDOWS
	LARGE_INTEGER current_ticks;
	QueryPerformanceCounter(&current_ticks);

	//Check if delta_ticks exceeds 1 seconds.
	//This is done to prevent possible problems caused by large time deltas,
	//which could be caused by checking breakpoints during development
	//or by occasional freezes during normal game-play.

	auto delta_microseconds = current_ticks.QuadPart - performanceCounterTicks;
	delta_microseconds *= 1000000; delta_microseconds /= processorFrequency;
	const auto delta_time = GTSL::Microseconds(delta_microseconds);
	
	if (delta_time > GTSL::Microseconds(GTSL::Seconds(1))) {
		deltaTime = static_cast<GTSL::Microseconds>(GTSL::Milliseconds(16));
	}
	else {
		deltaTime = delta_time;
	}
	
	elapsedTime += delta_time;
	
	//Set system ticks as this frame's ticks so in the next update we can work with it.
	performanceCounterTicks = current_ticks.QuadPart;
#elif BE_PLATFORM_LINUX
	timespec time;
	clock_gettime(CLOCK_MONOTONIC, &time);

	auto absoluteNanoseconds = GTSL::Nanoseconds(time.tv_nsec);
	const auto deltaTime = absoluteNanoseconds - elapsedTime;

	if(deltaTime > GTSL::Microseconds(GTSL::Seconds(1))) {
		this->deltaTime = static_cast<GTSL::Microseconds>(GTSL::Milliseconds(16));
	} else {
		this->deltaTime = deltaTime;
	}

	elapsedTime += deltaTime;
#endif
}


#undef GetCurrentTime
GTSL::Microseconds Clock::GetCurrentMicroseconds() const {
#if BE_PLATFORM_WINDOWS
	LARGE_INTEGER win_processor_ticks; QueryPerformanceCounter(&win_processor_ticks); return GTSL::Microseconds(win_processor_ticks.QuadPart * 1000000 / processorFrequency);
#elif BE_PLATFORM_LINUX
	timespec time; clock_gettime(CLOCK_MONOTONIC, &time); return GTSL::Microseconds(GTSL::Nanoseconds(time.tv_nsec));
#endif
}

//UTILITY GETTERS


GTSL::uint16 Clock::GetYear()
{
#if BE_PLATFORM_WINDOWS
	SYSTEMTIME WinTimeStructure;
	GetLocalTime(&WinTimeStructure);
	return WinTimeStructure.wYear;
#elif BE_PLATFORM_LINUX

	const time_t now = time(NULL);
	struct tm here;

	localtime_r(&now, &here);
	
	return here.tm_year + 1900;
#endif
	return 0;
}

Clock::Months Clock::GetMonth()
{
#if BE_PLATFORM_WINDOWS
	SYSTEMTIME WinTimeStructure;
	GetLocalTime(&WinTimeStructure);
	return static_cast<Months>(WinTimeStructure.wMonth);
#elif BE_PLATFORM_LINUX

	const time_t now = time(NULL);
	struct tm here;

	localtime_r(&now, &here);

	return static_cast<Months>(here.tm_mon + 1);
#endif
	return {};
}

GTSL::uint8 Clock::GetDayOfMonth()
{
#if BE_PLATFORM_WINDOWS
	SYSTEMTIME WinTimeStructure;
	GetLocalTime(&WinTimeStructure);
	return WinTimeStructure.wDay;
#elif BE_PLATFORM_LINUX

	const time_t now = time(NULL);
	struct tm here;

	localtime_r(&now, &here);

	return here.tm_mday;
#endif
	return 0;
}

Clock::Days Clock::GetDayOfWeek()
{
#if BE_PLATFORM_WINDOWS
	SYSTEMTIME WinTimeStructure;
	GetLocalTime(&WinTimeStructure);
	return (WinTimeStructure.wDayOfWeek == 0) ? Days::Sunday : static_cast<Days>(WinTimeStructure.wDayOfWeek);
#elif BE_PLATFORM_LINUX

	const time_t now = time(NULL);
	struct tm here;

	localtime_r(&now, &here);

	return static_cast<Days>(here.tm_wday);
#endif
	return {};
}

Clock::Time Clock::GetTime()
{
#if BE_PLATFORM_WINDOWS
	SYSTEMTIME WinTimeStructure;
	GetLocalTime(&WinTimeStructure);
	return { static_cast<GTSL::uint8>(WinTimeStructure.wHour), static_cast<GTSL::uint8>(WinTimeStructure.wMinute), static_cast<GTSL::uint8>(WinTimeStructure.wSecond) };
#elif BE_PLATFORM_LINUX

	const time_t now = time(NULL);
	struct tm here;

	localtime_r(&now, &here);

	return { static_cast<GTSL::uint8>(here.tm_hour), static_cast<GTSL::uint8>(here.tm_min), static_cast<GTSL::uint8>(here.tm_sec) };
#endif
	return {};
}
