#pragma once

#include "Core.h"

#include "EngineSystem.h"

//Used to specify time(Hour, Minute, Second).
struct Time
{
	unsigned short Hour;
	unsigned short Minute;
	unsigned short Second;
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
enum Months {
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
	static float GetDeltaTime();
	
	//Returns the delta time adjusted for time dilation.
	static float GetGameDeltaTime();
	
	//Returns the time the game has been running in real seconds.
	static float GetElapsedTime();
	
	//Returns the elapsed game time adjusted for time dilations and game pauses.
	static float GetElapsedGameTime();					


	//UTILITY GETTERS

	//Returns the current local year of the computer.
	static unsigned short GetYear();

	//Returns the current local month of the computer.
	static Months GetMonth();

	//Returns the current local day of the month of the computer.
	static unsigned short GetDayOfMonth();

	//Returns the current local day of the week of the computer.
	static Days GetDayOfWeek();

	//Returns the current local time (Hour, Minute, Second) of the computer.
	static Time GetTime();

	
	//CLOCK FUNCTIONALITY SETTERS

	static void SetTimeDilation(float Dilation);		//Sets the percentage by which time should be divided.
	static void SetIsPaused(bool IsPaused);

	// CALC

	inline float MilisecondsToSeconds(unsigned long long In) { return In / 1000.f; }
	inline float MicrosecondsToSeconds(unsigned long long In) { return In / 1000000.f; }
	inline float NanosecondsToSeconds(unsigned long long In) { return In / 1000000000.f; }

private:
	static bool ShouldUpdateGameTime;

	static float DeltaTime;								//Stores the real seconds elapsed since the last game update (tick).
	static unsigned long long SystemTicks;
	static float ElapsedTime;							//Stores the time the game has been running in real microseconds. 1,000,000.
	static float ElapsedGameTime;						//Stores the elapsed game time adjusted for time dilation and game pauses.
	static unsigned long long StartSystemTicks;

	static float TimeDivisor;							//Stores the percentage by which time should be divided.

	static unsigned long long ProcessorFrequency;		//Stores the frequency at which the processor operates. Used to calculate time differences between ticks.

	void SetDeltaTime();								//Sets DeltaTime.
};