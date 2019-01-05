#include "Logger.h"

#include "stdio.h"

#include "stdarg.h"

#include "windows.h"

#include "glad.h"

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

void Logger::PrintLog(const char * Text, LogColors Color, ...)
{
	SetLogTextColor(Color);

	SYSTEMTIME Time;

	GetLocalTime(&Time);

	printf("[Time: %02d:%02d:%02d]", Time.wHour, Time.wMinute, Time.wSecond);


	va_list args;
	va_start(args, Text);
	printf(Text, args);
	va_end(args);

	printf("\n");

	SetLogTextColor(White);
}

const char * Logger::GetglGetError()
{
	switch (glGetError())
	{
	case GL_NO_ERROR:
		return "No error.";
	case GL_INVALID_ENUM:
		return "Inavalid ENUM";
	}
	return "Exited prematurely!";
}