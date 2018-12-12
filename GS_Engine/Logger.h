#pragma once
#include <Windows.h>


//COLORS
//6 Yellow, 10 Light Green, 12 Bright Red, 15 White.

#define LOG_SUCCESS(Text) Print(Text, 10)
#define LOG_MESSAGE(Text) Print(Text, 15)
#define LOG_WARNING(Text) Print(Text, 6)
#define LOG_ERROR(Text) Print(Text, 12)

void PrintLog(const char* Text, int Color)
{
	#ifdef GS_PLATFORM_WIN
		//Set text color to that of the function's argument.
		SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), Color);

		//Create Variable to retrieve time.
		SYSTEMTIME Time;

		//Retrieve time.
		GetLocalTime(&Time);

		//Print whole message.
		printf("[Time: %02d:%02d:%02d] %s \n", Time.wHour, Time.wMinute, Time.wSecond, Text);

		//Set console text color back to white just in case.
		SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15);

	#endif // GS_PLATFORM_WIN

	#ifdef GS_PLATFORM_LINUX

	#endif // GS_PLATFORM_LINUX

	#ifdef GS_PLATFORM_MAC

	#endif // GS_PLATFORM_MAC


}
