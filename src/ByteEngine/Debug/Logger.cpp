#include "Logger.h"

#include "ByteEngine/Application/Clock.h"
#include "ByteEngine/Debug/FunctionTimer.h"

#include <GTSL/Console.h>

using namespace BE;

Logger::Logger(const LoggerCreateInfo& loggerCreateInfo) : Object(u8"Logger"), logFile()//, allowedLoggers(32, GetSyste())
{
	uint64 allocated_size{ 0 };
	GetPersistentAllocator().Allocate(defaultBufferLength, 1, reinterpret_cast<void**>(&data), &allocated_size);

	{
		GTSL::StaticString<260> path(loggerCreateInfo.AbsolutePathToLogDirectory);
		path += u8"/log.txt";
		switch (logFile.Open(path, GTSL::File::WRITE, true))
		{
		case GTSL::File::OpenResult::OK: break;
		case GTSL::File::OpenResult::CREATED: break;
		case GTSL::File::OpenResult::ERROR: break;
		default:;
		}
	}

	GTSL::StaticString<260> path(loggerCreateInfo.AbsolutePathToLogDirectory);
	path += u8"/trace.txt";
	switch (graphFile.Open(path, GTSL::File::WRITE, true))
	{
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: break;
	case GTSL::File::OpenResult::ERROR: break;
	default: ;
	}
	
	logFile.Resize(0);
	graphFile.Resize(0);

	{
		GTSL::StaticString<512> string;
		
		string += u8"{\"otherData\": {},\"traceEvents\":[";
		
		graphFile.Write(GTSL::Range(reinterpret_cast<const byte*>(string.begin()), reinterpret_cast<const byte*>(string.end() - 1)));
	}
}

void Logger::log(const VerbosityLevel verbosityLevel, const GTSL::Range<const char8_t*> text) const
{
	const auto day_of_month = Clock::GetDayOfMonth(); const auto month = Clock::GetMonth(); const auto year = Clock::GetYear(); const auto time = Clock::GetTime();

	GTSL::StaticString<maxLogLength> string;

	string += u8"[Date: ";
	ToString(day_of_month, string); string += u8"/";
	ToString(static_cast<uint8>(month), string); string += u8"/";
	ToString(year, string); string += u8"]";

	string += u8"[Time: ";
	ToString(time.Hour, string); string += u8":";
	ToString(time.Minute, string); string += u8":";
	ToString(time.Second, string); string += u8"] ";

	string += GTSL::Range<const utf8*>(GTSL::Math::Clamp(text.ElementCount(), 0ull, static_cast<uint64>(string.GetCapacity() - string.GetLength()) - 1), text.begin());// string += u8"\n";

	string += u8'\n';
	
	if(verbosityLevel >= minLogLevel) {
		SetTextColorOnLogLevel(verbosityLevel);
		WriteConsoleA(GetStdHandle(STD_OUTPUT_HANDLE), string.begin(), string.GetLength() - 1, nullptr, nullptr);
	}

	logMutex.Lock();
	logFile.Write(GTSL::Range<const byte*>(string.GetLength() - 1, reinterpret_cast<const byte*>(string.begin())));
	logMutex.Unlock();
}

void Logger::logFunctionTimer(FunctionTimer* functionTimer, GTSL::Microseconds timeTaken)
{
	GTSL::StaticString<1024> string;

	{
		GTSL::Lock lock(traceMutex);

		if (profileCount++ > 0)
			string += u8",";

		string += u8"{";
		string += u8"\"cat\":\"function\",";
		string += u8"\"dur\":"; GTSL::ToString(timeTaken.GetCount(), string); string += u8",";
		string += u8"\"name\":\""; string += functionTimer->Name; string += u8"\",";
		string += u8"\"ph\":\"X\",";
		string += u8"\"pid\":0,";
		string += u8"\"tid\":"; ToString(getThread(), string); string += u8",";
		string += u8"\"ts\":"; ToString(functionTimer->StartingTime, string);
		string += u8"}";

		graphFile.Write(GTSL::Range(reinterpret_cast<const byte*>(string.begin()), reinterpret_cast<const byte*>(string.end() - 1)));
	}
}

void Logger::SetTextColorOnLogLevel(const VerbosityLevel level) const
{
	switch (level)
	{
	case VerbosityLevel::MESSAGE: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::WHITE); break;
	case VerbosityLevel::SUCCESS: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::GREEN); break;
	case VerbosityLevel::WARNING: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::ORANGE); break;
	case VerbosityLevel::FATAL: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::RED); break;
	default: GTSL::Console::SetTextColor(GTSL::Console::ConsoleTextColor::WHITE); break;
	}
}

Logger::~Logger()
{
	{
		GTSL::StaticString<512> string;
		string += u8"]}";
		graphFile.Write(GTSL::Range(reinterpret_cast<const byte*>(string.begin()), reinterpret_cast<const byte*>(string.end()) - 1));
	}
}