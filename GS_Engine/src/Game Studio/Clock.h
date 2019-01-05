#pragma once

#include "Core.h"

#include "EngineSystem.h"

struct Time
{
	unsigned short Hour;
	unsigned short Minute;
	unsigned short Second;
};

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

GS_CLASS Clock : ESystem
{
public:
	Clock();
	~Clock();

	void OnUpdate();

	//CLOCK FUNCTIONALITY GETTERS

	float GetDeltaTime();					//Returns the real seconds elapsed since the last game update (tick).
	float GetElapsedTime();					//Returns the time the game has been running in real seconds.
	float GetElapsedGameTime();				//Returns the elapsed game time adjusted for time dilation and game pauses.


	//UTILITY GETTERS

	unsigned short GetYear();				//Returns the current local year of the computer.
	Months GetMonth();						//Returns the current local month of the computer.
	unsigned short GetDayOfMonth();			//Returns the current local day of the month of the computer.
	Days GetDayOfWeek();					//Returns the current local day of the week of the computer.
	Time GetTime();							//Returns the current local time (Hour, Minute, Second) of the computer.


	//CLOCK FUNCTIONALITY SETTERS

	void SetTimeDilation(float Dilation);	//Sets the percentage by which time should be divided.
	void SetIsPaused(bool IsPaused);

private:
	bool ShouldUpdateGameTime;

	float DeltaTime;						//Stores the real seconds elapsed since the last game update (tick).
	float ElapsedTime;						//Stores the time the game has been running in real seconds.
	float GameTime;							//Stores the elapsed game time adjusted for time dilation and game pauses.

	float TimeDivisor;						//Stores the percentage by which time should be divided.

	unsigned long  ProcessorFrequency;		//Stores the frequency at which the processor operates. Used to calculate time differences between ticks.


	//REAL-TIME TIME STORAGE
	unsigned short Year;
	Months Month;
	unsigned short DayOfMonth;
	Days DayOfWeek;
	Time Time;

	void  SetDeltaTime();					//Sets DeltaTime.
};