#pragma once

#include "Core.h"

enum LogColors
{
	Red,
	Yellow,
	Green,
	White
};


#ifdef GS_DEBUG
	#define GS_LOG_SUCCESS(Text, ...) Logger::PrintLog(Text, Green, __VA_ARGS__);
	#define GS_LOG_MESSAGE(Text, ...) Logger::PrintLog(Text, White, __VA_ARGS__);
	#define GS_LOG_WARNING(Text, ...) Logger::PrintLog(Text, Yellow, __VA_ARGS__);
	#define GS_LOG_ERROR(Text, ...) Logger::PrintLog(Text, Red, __VA_ARGS__);
#else
	#define GS_LOG_SUCCESS(Text, ...)
	#define GS_LOG_MESSAGE(Text, ...)
	#define GS_LOG_WARNING(Text, ...)
	#define GS_LOG_ERROR(Text, ...)
#endif

GS_CLASS Logger
{
public:
	static void PrintLog(const char * Text, LogColors Color, ...);
	static const char * GetglGetError();
private:
	static void SetLogTextColor(LogColors Color);
};