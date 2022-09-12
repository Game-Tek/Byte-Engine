#include "Logger.h"

#include "ByteEngine/Application/Clock.h"
#include "ByteEngine/Debug/FunctionTimer.h"

#include <GTSL/System.h>

#include "ByteEngine/Application/Application.h"

using namespace BE;

Logger::Logger(const LoggerCreateInfo& loggerCreateInfo) : Object(u8"Logger"), logFile()//, allowedLoggers(32, GetSyste())
{
	uint64 allocated_size{ 0 };
	GetPersistentAllocator().Allocate(defaultBufferLength, 1, reinterpret_cast<void**>(&data), &allocated_size);

	{
		GTSL::StaticString<260> path(loggerCreateInfo.AbsolutePathToLogDirectory);
		path += u8"/log.txt";
		switch (logFile.Open(path, GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		default:;
		}
	}
	
	logFile.Resize(0);

	GTSL::Console::SetConsoleInputModeAsUTF8();
}

void Logger::SetTrace(bool t) {
	trace = t;

	if (trace) {
		GTSL::StaticString<260> path(BE::Application::Get()->GetPathToApplication());
		path += u8"/trace.txt";
		switch (graphFile.Open(path, GTSL::File::WRITE, true)) {
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		default:;
		}
		graphFile.Resize(0);

		{
			GTSL::StaticString<512> string;

			string += u8"{\"otherData\": {},\"traceEvents\":[]}";

			graphFile.Write(GTSL::Range(string.GetBytes(), reinterpret_cast<const byte*>(string.c_str())));
		}
	}
}

//TODO: if string is too big clamp to nearest CODEPOINT not Byte

void Logger::log(const VerbosityLevel verbosityLevel, const GTSL::Range<const char8_t*> text) const
{
	const auto day_of_month = Clock::GetDayOfMonth(); const auto month = Clock::GetMonth(); const auto year = Clock::GetYear(); const auto time = Clock::GetTime();

	GTSL::StaticString<maxLogLength> string;

	string += u8"[";
	ToString(string, day_of_month); string += u8"/";
	ToString(string, static_cast<uint8>(month)); string += u8"/";
	ToString(string, year); string += u8"]";

	string += u8"[";
	ToString(string, time.Hour); string += u8":";
	ToString(string, time.Minute); string += u8":";
	ToString(string, time.Second); string += u8"] ";

	auto clampedSize = GTSL::Math::Limit(text.GetBytes(), string.GetCapacity() - string.GetBytes() - 1);
	string += GTSL::Range<const utf8*>(clampedSize, clampedSize, text.GetData());

	string += u8'\n';
	
	if(verbosityLevel >= minLogLevel) {
		SetTextColorOnLogLevel(verbosityLevel);
		GTSL::Console::Print(string);
	}

	logMutex.Lock();
	logFile.Write(GTSL::Range<const byte*>(string.GetBytes(), reinterpret_cast<const byte*>(string.c_str())));
	logMutex.Unlock();
}

void Logger::SetTextColorOnLogLevel(const VerbosityLevel level) const
{
	switch (level) {
	case VerbosityLevel::MESSAGE: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::WHITE); break;
	case VerbosityLevel::SUCCESS: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::GREEN); break;
	case VerbosityLevel::WARNING: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::ORANGE); break;
	case VerbosityLevel::FATAL: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::RED); break;
	default: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::WHITE); break;
	}
}

Logger::~Logger()
{
}