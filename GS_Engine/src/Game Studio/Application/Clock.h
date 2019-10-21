#pragma once

#include "Core.h"

#include "Object.h"

using Nanoseconds = uint_64;
using Microseconds = uint_64;
using Miliseconds = uint_64;

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

class GS_API Clock : public Object
{
public:
	Clock();
	~Clock();

	void OnUpdate() override;
	[[nodiscard]] const char* GetName() const override { return "Clock"; }

	//CLOCK FUNCTIONALITY GETTERS

	//Returns the real seconds elapsed since the last game update (tick).
	[[nodiscard]] double GetDeltaTime() const;
	
	//Returns the delta time adjusted for time dilation.
	[[nodiscard]] double GetGameDeltaTime() const;
	
	//Returns the time the game has been running in real seconds.
	[[nodiscard]] double GetElapsedTime() const;
	
	//Returns the elapsed game time adjusted for time dilations and game pauses.
	[[nodiscard]] double GetElapsedGameTime() const;

	[[nodiscard]] uint_64 GetGameTicks() const { return GameTicks; }

	[[nodiscard]] Nanoseconds GetCurrentNanoseconds() const;

	[[nodiscard]] double GetFPS() const { return 1.0 / GetDeltaTime(); }
	[[nodiscard]] double GetGameFPS() const { return 1.0 / GetGameDeltaTime(); }
	
	//UTILITY GETTERS

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

	
	//CLOCK FUNCTIONALITY SETTERS

	//Sets the percentage by which time should be divided.
	void SetTimeDilation(double _Dilation) { TimeDivisor = _Dilation; }

	//Sets if (game)time should be updated.
	void SetIsPaused(bool _IsPaused) { ShouldUpdateGameTime = _IsPaused; }

	// CALC

	static INLINE double MilisecondsToSeconds(const Miliseconds _In) { return _In / 1000; }
	static INLINE double MicrosecondsToSeconds(const Microseconds _In) { return _In / 1000000; }
	static INLINE double NanosecondsToSeconds(const Nanoseconds _In) { return _In / 1000000000; }
	static INLINE uint16 SecondsToFPS(const float _Seconds) { return 1 / _Seconds; }

private:
	bool ShouldUpdateGameTime = true;

	double DeltaTime = 0.0f;							//Stores the real seconds elapsed since the last game update (tick).

	uint_64 GameTicks = 0;
	double ElapsedTime = 0.0f;						//Stores the time the game has been running in real microseconds. 1,000,000.
	double ElapsedGameTime = 0.0f;					//Stores the elapsed game time adjusted for time dilation and game pauses.

	double TimeDivisor = 1.0f;						//Stores the percentage by which time should be divided.

	uint_64 StartSystemTicks = 0;
	uint_64 SystemTicks = 0;
	uint_64 ProcessorFrequency = 0;					//Stores the frequency at which the processor operates. Used to calculate time differences between ticks.
};