#pragma once

#include "Core.h"

enum LogColors
{
	Red,
	Yellow,
	Green,
	White
};

GS_CLASS Logger
{
	#ifdef GS_DEBUG
#define GS_LOG_SUCCESS(Text, ...)	Logger::SetLogTextColor(Green);\
									Logger::PrintLog(Text, __VA_ARGS__);\

#define GS_LOG_MESSAGE(Text, ...)	Logger::SetLogTextColor(White);\
									Logger::PrintLog(Text, __VA_ARGS__);\

#define GS_LOG_WARNING(Text, ...)	Logger::SetLogTextColor(Yellow);\
									Logger::PrintLog(Text, __VA_ARGS__);\

#define GS_LOG_ERROR(Text, ...)		Logger::SetLogTextColor(Red);\
									Logger::PrintLog(Text, __VA_ARGS__);\
									
#else
	#define GS_LOG_SUCCESS(Text, ...)
	#define GS_LOG_MESSAGE(Text, ...)
	#define GS_LOG_WARNING(Text, ...)
	#define GS_LOG_ERROR(Text, ...)
#endif

public:
	static void PrintLog(const char * Text, ...);
	static void SetLogTextColor(LogColors Color);
private:

};