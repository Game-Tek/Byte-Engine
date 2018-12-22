#pragma once

#include <Windows.h>

#include "Clock.h"


//COLORS
//6 Yellow, 10 Light Green, 12 Bright Red, 15 White.

#define GS_LOG_SUCCESS(Text) PrintLog(Text, 10);
#define GS_LOG_MESSAGE(Text) PrintLog(Text, 15);
#define GS_LOG_WARNING(Text) PrintLog(Text, 6);
#define GS_LOG_ERROR(Text)   PrintLog(Text, 12);

void SetLogTextColor(int Color)
{
	SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), Color);

	return;
}

void PrintLog(const char* Text, int Color)
{
	SetLogTextColor(Color);

	//Print whole message.
	//printf("[Time: %02d:%02d:%02d] %s \n", Time.Hour, Time.Minute, Time.Second, Text);

	//Set console text color back to white just in case.
	SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15);

	return;
}