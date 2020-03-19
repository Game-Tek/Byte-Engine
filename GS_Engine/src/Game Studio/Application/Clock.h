#pragma once

#include "Core.h"

#include "Object.h"
#include "Containers/TimePoint.h"

//Used to specify time(Hour, Minute, Second).
struct Time
{
	uint8 Hour;
	uint8 Minute;
	uint8 Second;
};

//Used to specify days of the week, with Monday being 1 and Sunday being 7.
enum class Days : uint8
{
	Monday = 1,
	Tuesday,
	Wednesday,
	Thursday,
	Friday,
	Saturday,
	Sunday,
};

//Used to specify months, with January being 1 and December being 12.
enum class Months : uint8
{
	January = 1,
	February,
	March,
	April,
	May,
	June,
	July,
	August,
	September,
	October,
	November,
	December,
};


class Clock : public Object
{
public:
	Clock();
	~Clock();

	void OnUpdate();
	
	[[nodiscard]] const char* GetName() const override { return "Clock"; }

	//Returns the real seconds elapsed since the last game update (tick).
	[[nodiscard]] TimePoint GetDeltaTime() const { return deltaTime; }

	//Returns the time the game has been running in real seconds.
	[[nodiscard]] TimePoint GetElapsedTime() const { return elapsedTime; }

	[[nodiscard]] uint_64 GetApplicationTicks() const { return applicationTicks; }

	[[nodiscard]] TimePoint GetCurrentTime() const;

	//Returns the current local year of the computer.
	static uint16 GetYear();

	//Returns the current local month of the computer.
	static Months GetMonth();

	//Returns the current local day of the month of the computer.
	static uint8 GetDayOfMonth();

	//Returns the current local day of the week of the computer.
	static Days GetDayOfWeek();

	//Returns the current local time (Hour, Minute, Second) of the computer.
	static Time GetTime();

private:
	uint_64 applicationTicks = 0;

	uint_64 startPerformanceCounterTicks = 0;
	uint_64 performanceCounterTicks = 0;
	//Stores the frequency at which the processor operates. Used to calculate time differences between ticks.
	uint_64 processorFrequency = 0;

	TimePoint deltaTime;
	TimePoint elapsedTime;
};
