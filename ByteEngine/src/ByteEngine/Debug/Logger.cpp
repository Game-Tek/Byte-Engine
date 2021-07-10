#include "Logger.h"

#include <GTSL/Thread.h>

#include "ByteEngine/Application/Clock.h"
#include "ByteEngine/Debug/FunctionTimer.h"

using namespace BE;

Logger::Logger(const LoggerCreateInfo& loggerCreateInfo) : Object(u8"Logger"), logFile()
{
	uint64 allocated_size{ 0 };
	GetPersistentAllocator().Allocate(defaultBufferLength, 1, reinterpret_cast<void**>(&data), &allocated_size);
	
	GTSL::StaticString<260> path(loggerCreateInfo.AbsolutePathToLogDirectory);
	path += u8"/log.txt";
	switch (logFile.Open(path, GTSL::File::WRITE, true))
	{
	case GTSL::File::OpenResult::OK: break;
	case GTSL::File::OpenResult::CREATED: break;
	case GTSL::File::OpenResult::ERROR: break;
	default: ;
	}
	
	logFile.Resize(0);
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
	//log(VerbosityLevel::MESSAGE, GTSL::Range<UTF8>(GTSL::StringLength(functionTimer->Name), functionTimer->Name));
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
}