#pragma once

#include "Core.h"

enum class LogLevel : uint8
{
	MESSAGE,
	SUCCESS,
	WARNING,
	ERROR
};

GS_CLASS Logger
{
	static LogLevel MinLogLevel;

	static void SetTextColorOnLogLevel(LogLevel _Level);
public:
	static void PrintLog(LogLevel _Level, const char * Text, ...);
	static void SetMinLogLevel(LogLevel _Level) { MinLogLevel = _Level; }

#ifdef GS_DEBUG

#define GS_LOG_SUCCESS(Text, ...)	Logger::PrintLog(LogLevel::SUCCESS, Text, __VA_ARGS__);\

#define GS_LOG_MESSAGE(Text, ...)	Logger::PrintLog(LogLevel::MESSAGE, Text, __VA_ARGS__);\

#define GS_LOG_WARNING(Text, ...)	Logger::PrintLog(LogLevel::WARNING, Text, __VA_ARGS__);\

#define GS_LOG_ERROR(Text, ...)		Logger::PrintLog(LogLevel::ERROR, Text, __VA_ARGS__);\

#define GS_LOG_LEVEL(_Level)		Logger::SetMinLogLevel(_Level);

#else

#define GS_LOG_SUCCESS(Text, ...)
#define GS_LOG_MESSAGE(Text, ...)
#define GS_LOG_WARNING(Text, ...)
#define GS_LOG_ERROR(Text, ...)
#define GS_LOG_LEVEL(_Level)

#endif
};

