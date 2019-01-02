#pragma once

#include "Clock.h"

enum LogColors
{
	Red,
	Yellow,
	Green,
	White
};

//COLORS
//6 Yellow, 10 Light Green, 12 Bright Red, 15 White.

#define GS_LOG_SUCCESS(Text) PrintLog(Text, Green);
#define GS_LOG_MESSAGE(Text) PrintLog(Text, White);
#define GS_LOG_WARNING(Text) PrintLog(Text, Yellow);
#define GS_LOG_ERROR(Text) PrintLog(Text, Red);

void SetLogTextColor(LogColors Color)
{
	switch (Color)
	{
	Red:	
			SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 12);
			break;

	Yellow:	
			SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 6);
			break;

	Green:	
			SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 10);
			break;

	White:
			SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15);
			break;
	}

	return;
}

void PrintLog(const char* Text, LogColors Color)
{
	SetLogTextColor(Color);

	//Print whole message.
	//printf("[Time: %02d:%02d:%02d] %s \n", Time.Hour, Time.Minute, Time.Second, Text);

	//Set console text color back to white just in case.
	SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15);

	return;
}