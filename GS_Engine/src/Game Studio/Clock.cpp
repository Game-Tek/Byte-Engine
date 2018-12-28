#pragma once

#include "Clock.h"

#include "windows.h"

Clock::Clock()
{
#ifdef GS_PLATFORM_WIN
	LARGE_INTEGER WinProcessorFrequency;

	QueryPerformanceFrequency(& WinProcessorFrequency);

	ProcessorFrequency = (unsigned long)WinProcessorFrequency.QuadPart;


	SYSTEMTIME WinTimeStructure;

	GetLocalTime(& WinTimeStructure);

	Year = WinTimeStructure.wYear;

	Month = (Months)WinTimeStructure.wMonth;

	DayOfMonth = WinTimeStructure.wDay;

	DayOfWeek = (WinTimeStructure.wDayOfWeek == 0) ? Sunday : (Days)WinTimeStructure.wDayOfWeek;
#endif
}

Clock::~Clock()
{
}

void Clock::OnUpdate()
{
}

//CLOCK FUNCTIONALITY GETTERS

float Clock::GetDeltaTime()
{
	return DeltaTime;
}

float Clock::GetElapsedTime()
{
	return ElapsedTime;
}

float Clock::GetElapsedGameTime()
{
	return GameTime;
}


//UTILITY GETTERS

unsigned short Clock::GetYear()
{
	return Year;
}

Months Clock::GetMonth()
{
	return Month;
}

unsigned short Clock::GetDayOfMonth()
{
	return DayOfMonth;
}

Days Clock::GetDayOfWeek()
{
	return DayOfWeek;
}

Time Clock::GetTime()
{
	return Time;
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
