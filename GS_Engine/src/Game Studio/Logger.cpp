#include "Logger.h"

#include <cstdio>

#include <windows.h>

#include "Application/Application.h"

void Logger::SetLogTextColor(LogColors Color)
{
	switch (Color)
	{
	default:
			SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15);
			break;

		case Red:
			SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 12);
			break;

		case Yellow:
			SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 6);
			break;

		case Green:
			SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 10);
			break;

		case White:
			SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15);
			break;
	}
	return;
}

void Logger::PrintLog(const char * Text, ...)
{
	//SetLogTextColor(Color);

	const Time LogTime = GS::Application::Get()->GetClockInstance()->GetTime();

	printf("[Time: %02d:%02d:%02d]", LogTime.Hour, LogTime.Minute, LogTime.Second);

	va_list args;
	va_start(args, Text);
	vprintf(Text, args);
	va_end(args);

	printf("\n");

	SetLogTextColor(White);
}