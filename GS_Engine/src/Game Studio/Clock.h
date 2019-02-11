#pragma once

#include "Core.h"

#include "EngineSystem.h"

//Used to specify time(Hour, Minute, Second).
struct Time
{
	uint8 Hour;
	uint8 Minute;
	uint8 Second;
};

//Used to specify days of the week, with Monday being 1 and Sunday being 7.
enum Days
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
enum Months
{
	January = 1,
	February,
	March,
	April,
	May,
	June,
	July,
	August,
	Semptember,
	October,
	November,
	December,
};

GS_CLASS Clock : public ESystem
{
public:
	Clock();
	~Clock();

	void OnUpdate() override;

	//CLOCK FUNCTIONALITY GETTERS

	//Returns the real seconds elapsed since the last game update (tick).
	float GetDeltaTime() const;
	
	//Returns the delta time adjusted for time dilation.
	float GetGameDeltaTime() const;
	
	//Returns the time the game has been running in real seconds.
	float GetElapsedTime() const;
	
	//Returns the elapsed game time adjusted for time dilations and game pauses.
	float GetElapsedGameTime() const;					


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
	void SetTimeDilation(float Dilation);

	//Sets if (game)time should be updated.
	void SetIsPaused(bool IsPaused);

	// CALC

	static INLINE float MilisecondsToSeconds(const uint_64 In) { return In / 1000.f; }
	static INLINE float MicrosecondsToSeconds(const uint_64 In) { return In / 1000000.f; }
	static INLINE float NanosecondsToSeconds(const uint_64 In) { return In / 1000000000.f; }

private:
	bool ShouldUpdateGameTime = true;

	float DeltaTime = 0.0f;							//Stores the real seconds elapsed since the last game update (tick).
	uint_64 SystemTicks = 0;
	float ElapsedTime = 0.0f;						//Stores the time the game has been running in real microseconds. 1,000,000.
	float ElapsedGameTime = 0.0f;					//Stores the elapsed game time adjusted for time dilation and game pauses.
	uint_64 StartSystemTicks = 0;

	float TimeDivisor = 1.0f;						//Stores the percentage by which time should be divided.

	uint_64 ProcessorFrequency = 0;					//Stores the frequency at which the processor operates. Used to calculate time differences between ticks.
};