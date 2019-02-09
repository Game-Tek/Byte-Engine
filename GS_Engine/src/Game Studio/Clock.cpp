#pragma once

#include "Clock.h"

#include "windows.h"

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
	LARGE_INTEGER WinProcessorTicks;

	QueryPerformanceCounter(&WinProcessorTicks);

	unsigned long long Delta = WinProcessorTicks.QuadPart - SystemTicks;


	//Calculate delta time.
	float loc_DeltaTime = Delta / (float)ProcessorFrequency;


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

	return;
}

//CLOCK FUNCTIONALITY GETTERS

float Clock::GetDeltaTime()
{
	return DeltaTime;
}

float Clock::GetGameDeltaTime()
{
	return DeltaTime * TimeDivisor;
}

float Clock::GetElapsedTime()
{
	return (SystemTicks - StartSystemTicks) / (float)ProcessorFrequency;
}

float Clock::GetElapsedGameTime()
{
	return ElapsedGameTime;
}

//UTILITY GETTERS

unsigned short Clock::GetYear()
{
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(&WinTimeStructure);

	return WinTimeStructure.wYear;
}

Months Clock::GetMonth()
{
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(&WinTimeStructure);

	return (Months)WinTimeStructure.wMonth;
}

unsigned short Clock::GetDayOfMonth()
{
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(&WinTimeStructure);

	return WinTimeStructure.wDay;
}

Days Clock::GetDayOfWeek()
{
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(& WinTimeStructure);

	return (WinTimeStructure.wDayOfWeek == 0) ? Sunday : (Days)WinTimeStructure.wDayOfWeek;
}

Time Clock::GetTime()
{
	SYSTEMTIME WinTimeStructure;

	GetLocalTime(& WinTimeStructure);
	
	return { (uint8)WinTimeStructure.wHour, (uint8)WinTimeStructure.wMinute, (uint8)WinTimeStructure.wSecond };
}

//CLOCK FUNCTIONALITY

void Clock::SetTimeDilation(float Dilation)
{
	TimeDivisor = Dilation;

	return;
}

void Clock::SetIsPaused(bool IsPaused)
{
	ShouldUpdateGameTime = IsPaused;
	
	return;
}