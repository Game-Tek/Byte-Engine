#pragma once

#include "ByteEngine/Core.h"
#include "ByteEngine/Object.h"
#include <GTSL/Time.h>


class Clock : public Object {
public:
	Clock();
	~Clock();

	void OnUpdate();

	//Used to specify time(Hour, Minute, Second).
	struct Time {
		GTSL::uint8 Hour;
		GTSL::uint8 Minute;
		GTSL::uint8 Second;
	};

	//Used to specify days of the week, with Monday being 1 and Sunday being 7.
	enum class Days : GTSL::uint8 {
		Monday = 1,
		Tuesday,
		Wednesday,
		Thursday,
		Friday,
		Saturday,
		Sunday,
	};

	//Used to specify months, with January being 1 and December being 12.
	enum class Months : GTSL::uint8 {
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

	//Returns the time elapsed since the last application update (tick).
	[[nodiscard]] GTSL::Microseconds GetDeltaTime() const { return deltaTime; }

	//Returns the time the game has been running.
	[[nodiscard]] GTSL::Microseconds GetElapsedTime() const { return elapsedTime; }

	[[nodiscard]] GTSL::Microseconds GetCurrentMicroseconds() const;

	//Returns the current local year of the computer.
	static GTSL::uint16 GetYear();

	//Returns the current local month of the computer.
	static Months GetMonth();

	//Returns the current local day of the month of the computer.
	static GTSL::uint8 GetDayOfMonth();

	//Returns the current local day of the week of the computer.
	static Days GetDayOfWeek();

	//Returns the current local time (Hour, Minute, Second) of the computer.
	static Time GetTime();

private:
#if BE_PLATFORM_WINDOWS
	GTSL::uint64 startPerformanceCounterTicks = 0;
	GTSL::uint64 performanceCounterTicks = 0;
	//Stores the frequency at which the processor operates. Used to calculate time differences between ticks.
	GTSL::uint64 processorFrequency = 0;
#endif

	GTSL::Microseconds deltaTime;
	GTSL::Microseconds elapsedTime;
};
