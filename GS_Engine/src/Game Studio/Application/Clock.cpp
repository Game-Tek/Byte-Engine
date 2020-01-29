#include "Clock.h"

#ifdef GS_PLATFORM_WIN
#include "windows.h"
#endif

Clock::Clock()
{
#ifdef GS_PLATFORM_WIN
	LARGE_INTEGER WinProcessorFrequency;
	LARGE_INTEGER WinProcessorTicks;

	QueryPerformanceFrequency(&WinProcessorFrequency);
	QueryPerformanceCounter(&WinProcessorTicks);

	ProcessorFrequency = WinProcessorFrequency.QuadPart;
	StartSystemTicks = WinProcessorTicks.QuadPart;
	SystemTicks = WinProcessorTicks.QuadPart;
#endif
}

Clock::~Clock()
{
}

void Clock::OnUpdate()
{
#ifdef GS_PLATFORM_WIN
	LARGE_INTEGER win_processor_ticks;

	QueryPerformanceCounter(&win_processor_ticks);

	const uint_64 delta = win_processor_ticks.QuadPart - SystemTicks;


	//Calculate delta time.
	const auto delta_time = delta / static_cast<double>(ProcessorFrequency);


	//Check if loc_DeltaTime exceed 1 seconds.
	//This is done to prevent possible problems caused by large time deltas,
	//which could be caused by checking breakpoints during development
	//or by occasional freezes during normal game-play.

	if (delta_time > 1.0)
	{
		//Leave delta time as is. Assume last frame's delta time.
	}
	else
	{
		//If loc_DeltaTime is less than one second set DeltaTime as loc_DeltaTime.
		DeltaTime = delta_time;
	}

	//Set system ticks as this frame's ticks so in the next update we can work with it.
	SystemTicks = win_processor_ticks.QuadPart;

	//Update elapsed time counter.
	ElapsedTime += DeltaTime;

	//Update elapsed game time counter.
	ElapsedGameTime += GetGameDeltaTime();
#endif

	++GameTicks;

	return;
}

//CLOCK FUNCTIONALITY GETTERS

double Clock::GetDeltaTime() const
{
	return DeltaTime;
}

double Clock::GetGameDeltaTime() const
{
	return DeltaTime * TimeDivisor * ShouldUpdateGameTime;
}

double Clock::GetElapsedTime() const
{
	return (SystemTicks - StartSystemTicks) / static_cast<double>(ProcessorFrequency);
}

double Clock::GetElapsedGameTime() const
{
	return ElapsedGameTime;
}

//UTILITY GETTERS

Nanoseconds Clock::GetCurrentNanoseconds() const
{
	LARGE_INTEGER WinProcessorTicks;

	QueryPerformanceCounter(&WinProcessorTicks);

	return (WinProcessorTicks.QuadPart * 1000000000) / ProcessorFrequency;
}

uint16 Clock::GetYear()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(&WinTimeStructure);

	return WinTimeStructure.wYear;
#endif
}

Months Clock::GetMonth()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(&WinTimeStructure);

	return static_cast<Months>(WinTimeStructure.wMonth);
#endif
}

uint8 Clock::GetDayOfMonth()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(&WinTimeStructure);

	return WinTimeStructure.wDay;
#endif
}

Days Clock::GetDayOfWeek()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(&WinTimeStructure);

	return (WinTimeStructure.wDayOfWeek == 0) ? Days::Sunday : static_cast<Days>(WinTimeStructure.wDayOfWeek);
#endif
}

Time Clock::GetTime()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(&WinTimeStructure);

	return {
		static_cast<uint8>(WinTimeStructure.wHour), static_cast<uint8>(WinTimeStructure.wMinute),
		static_cast<uint8>(WinTimeStructure.wSecond)
	};
#endif
}
