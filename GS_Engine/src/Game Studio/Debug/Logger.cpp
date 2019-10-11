#include "Logger.h"

#include <cstdio>

#include <windows.h>

#include "Application/Application.h"

LogLevel Logger::MinLogLevel;

void Logger::PrintObjectLog(const Object* _Obj, LogLevel _Level, const char* Text, ...)
{
	if(_Level >= MinLogLevel)
	{
		SetTextColorOnLogLevel(_Level);

		const Time LogTime = Clock::GetTime();

		printf("[Time: %02d:%02d:%02d]", LogTime.Hour, LogTime.Minute, LogTime.Second);
		printf("%s: ", _Obj->GetName());

		va_list args;
		va_start(args, Text);
		vprintf(Text, args);
		va_end(args);

		printf("\n");
	}
}

void Logger::SetTextColorOnLogLevel(LogLevel _Level)
{
	switch (_Level)
	{
	default:
		SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15);
		break;

	case LogLevel::MESSAGE:
		SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15);
		break;

	case LogLevel::SUCCESS:
		SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), FOREGROUND_GREEN | FOREGROUND_INTENSITY);
		break;

	case LogLevel::WARNING:
		SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), FOREGROUND_RED | FOREGROUND_GREEN);
		break;

	#undef ERROR

	case LogLevel::ERROR:
		SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), FOREGROUND_RED | FOREGROUND_INTENSITY);
		break;

	}
	return;
}

void Logger::PrintBasicLog(LogLevel _Level, const char* Text, ...)
{
	if (_Level >= MinLogLevel)
	{
		SetTextColorOnLogLevel(_Level);

		const Time LogTime = Clock::GetTime();

		printf("[Time: %02d:%02d:%02d]", LogTime.Hour, LogTime.Minute, LogTime.Second);

		va_list args;
		va_start(args, Text);
		vprintf(Text, args);
		va_end(args);

		printf("\n");
	}
}
