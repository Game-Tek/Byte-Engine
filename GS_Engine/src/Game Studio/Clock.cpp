#include "Clock.h"

#ifdef GS_PLATFORM_WIN
#include "windows.h"
#endif

Clock::Clock()
{
#ifdef GS_PLATFORM_WIN
	LARGE_INTEGER WinProcessorFrequency;
	LARGE_INTEGER WinProcessorTicks;

	QueryPerformanceFrequency(& WinProcessorFrequency);
	QueryPerformanceCounter(& WinProcessorTicks);

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
	LARGE_INTEGER WinProcessorTicks;

	QueryPerformanceCounter(&WinProcessorTicks);

	const uint_64 Delta = WinProcessorTicks.QuadPart - SystemTicks;


	//Calculate delta time.
	const float loc_DeltaTime = Delta / static_cast<float>(ProcessorFrequency);


	//Check if loc_DeltaTime exceed 1 seconds.
	//This is done to prevent possible problems caused by large time deltas,
	//which could be caused by checking breakpoints during development
	//or by ocassional freezes during normal gameplay.

	if (loc_DeltaTime > 1.0f)
	{
		DeltaTime = 0.01666f;
	}

	//If loc_DeltaTime is less than one second set DeltaTime as loc_DeltaTime.
	DeltaTime = loc_DeltaTime;

	//Set system ticks as this frame's ticks so in the next update we can work with it.
	SystemTicks = WinProcessorTicks.QuadPart;

	//Update elpased time counter.
	ElapsedTime += DeltaTime;

	//Update elapsed game time counter.
	ElapsedGameTime += GetGameDeltaTime();
#endif

	return;
}

//CLOCK FUNCTIONALITY GETTERS

float Clock::GetDeltaTime() const
{
	return DeltaTime;
}

float Clock::GetGameDeltaTime() const
{
	return DeltaTime * TimeDivisor * ShouldUpdateGameTime;
}

float Clock::GetElapsedTime() const
{
	return (SystemTicks - StartSystemTicks) / static_cast<float>(ProcessorFrequency);
}

float Clock::GetElapsedGameTime() const
{
	return ElapsedGameTime;
}

//UTILITY GETTERS

uint16 Clock::GetYear()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(& WinTimeStructure);

	return WinTimeStructure.wYear;
#endif
}

Months Clock::GetMonth()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(& WinTimeStructure);

	return static_cast<Months>(WinTimeStructure.wMonth);
#endif
}

uint8 Clock::GetDayOfMonth()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(& WinTimeStructure);

	return WinTimeStructure.wDay;
#endif
}

Days Clock::GetDayOfWeek()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(& WinTimeStructure);

	return (WinTimeStructure.wDayOfWeek == 0) ? Sunday : static_cast<Days>(WinTimeStructure.wDayOfWeek);
#endif
}

Time Clock::GetTime()
{
#ifdef GS_PLATFORM_WIN
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(& WinTimeStructure);
	
	return { static_cast<uint8>(WinTimeStructure.wHour), static_cast<uint8>(WinTimeStructure.wMinute), static_cast<uint8>(WinTimeStructure.wSecond) };
#endif
}

//CLOCK FUNCTIONALITY

void Clock::SetTimeDilation(const float Dilation)
{
	TimeDivisor = Dilation;

	return;
}

void Clock::SetIsPaused(const bool IsPaused)
{
	ShouldUpdateGameTime = IsPaused;
	
	return;
}