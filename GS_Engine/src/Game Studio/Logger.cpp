#include "Logger.h"

#include "stdio.h"

#include "windows.h"

#include "GLAD/glad.h"

#include "Application.h"

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

	Time LogTime = GS::Application::GetClockInstance()->GetTime();

	printf("[Time: %02d:%02d:%02d]", LogTime.Hour, LogTime.Minute, LogTime.Second);

	va_list args;
	va_start(args, Text);
	vprintf(Text, args);
	va_end(args);

	printf("\n");

	SetLogTextColor(White);
}

void Logger::GetglGetError(const char* Details)
{
	switch (glGetError())
	{
	default:
		break;
	case GL_NO_ERROR:
		break;
	case GL_INVALID_ENUM:
		GS_LOG_ERROR("Invalid enum!");
		break;
	case GL_INVALID_VALUE:
		GS_LOG_ERROR("Inavlid Value!");
		break;
	case GL_INVALID_OPERATION:
		GS_LOG_ERROR("Invalid Operation!");
		break;
	case GL_OUT_OF_MEMORY:
		GS_LOG_ERROR("Out of Memory!");
		break;
	}

	//GS_LOG_ERROR(Details)
	return;
}