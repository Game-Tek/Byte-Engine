#pragma once

#include "Core.h"

class Object;

#undef ERROR

enum class LogLevel : uint8
{
	MESSAGE,
	SUCCESS,
	WARNING,
	FATAL
};

class GS_API Logger
{
	static LogLevel MinLogLevel;

	static void SetTextColorOnLogLevel(LogLevel _Level);
public:
	static void PrintObjectLog(const Object* _Obj, LogLevel _Level, const char* Text, ...);
	static void PrintBasicLog(LogLevel _Level, const char* Text, ...);
	static void SetMinLogLevel(LogLevel _Level) { MinLogLevel = _Level; }

#ifdef GS_DEBUG

#define GS_LOG_SUCCESS(Text, ...)	Logger::PrintObjectLog(this, LogLevel::SUCCESS, Text, __VA_ARGS__);\

#define GS_LOG_MESSAGE(Text, ...)	Logger::PrintObjectLog(this, LogLevel::MESSAGE, Text, __VA_ARGS__);\

#define GS_LOG_WARNING(Text, ...)	Logger::PrintObjectLog(this, LogLevel::WARNING, Text, __VA_ARGS__);\

#define GS_LOG_ERROR(Text, ...)		Logger::PrintObjectLog(this, LogLevel::FATAL, Text, __VA_ARGS__);\

#define GS_LOG_LEVEL(_Level)		Logger::SetMinLogLevel(_Level);

#define GS_BASIC_LOG_SUCCESS(Text, ...)	Logger::PrintBasicLog(LogLevel::SUCCESS, Text, __VA_ARGS__);\
													 
#define GS_BASIC_LOG_MESSAGE(Text, ...)	Logger::PrintBasicLog(LogLevel::MESSAGE, Text, __VA_ARGS__);\
													 
#define GS_BASIC_LOG_WARNING(Text, ...)	Logger::PrintBasicLog(LogLevel::WARNING, Text, __VA_ARGS__);\
													 
#define GS_BASIC_LOG_ERROR(Text, ...)	Logger::PrintBasicLog(LogLevel::FATAL, Text, __VA_ARGS__);\

#else

#define GS_LOG_SUCCESS(Text, ...)
#define GS_LOG_MESSAGE(Text, ...)
#define GS_LOG_WARNING(Text, ...)
#define GS_LOG_ERROR(Text, ...)
#define GS_LOG_LEVEL(_Level)
#define GS_BASIC_LOG_SUCCESS(Text, ...)	
#define GS_BASIC_LOG_MESSAGE(Text, ...)	
#define GS_BASIC_LOG_WARNING(Text, ...)	
#define GS_BASIC_LOG_ERROR(Text, ...)	

#endif
};

