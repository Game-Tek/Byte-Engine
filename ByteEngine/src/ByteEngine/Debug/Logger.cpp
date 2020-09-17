#include "Logger.h"

#include <cstdio>
#include <GTSL/Memory.h>
#include <GTSL/Thread.h>


#include "ByteEngine/Application/Clock.h"
#include "ByteEngine/Debug/FunctionTimer.h"

using namespace BE;

Logger::Logger(const LoggerCreateInfo& loggerCreateInfo) : Object("Logger"), logFile()
{
	uint64 allocated_size{ 0 };
	GetPersistentAllocator().Allocate(defaultBufferLength, 1, reinterpret_cast<void**>(&data), &allocated_size);
	
	GTSL::SetMemory(defaultBufferLength, data, 'o');
	
	GTSL::StaticString<260> path(loggerCreateInfo.AbsolutePathToLogDirectory);
	path += "/log.txt";
	logFile.OpenFile(path, (uint8)GTSL::File::AccessMode::WRITE, GTSL::File::OpenMode::CLEAR);
}

void Logger::Shutdown() const
{
	//logMutex.Lock();
	//auto range = GTSL::Ranger<const byte>(reinterpret_cast<byte*>(data) + buffersInBuffer * subBufferIndex, reinterpret_cast<byte*>(data) + posInBuffer + buffersInBuffer * subBufferIndex);
	//logFile.WriteToFile(range);
	//logMutex.Unlock();
	logFile.CloseFile();
}

void Logger::log(const VerbosityLevel verbosityLevel, const GTSL::Ranger<const GTSL::UTF8>& text) const
{
	const auto day_of_month = Clock::GetDayOfMonth(); const auto month = Clock::GetMonth(); const auto year = Clock::GetYear(); const auto time = Clock::GetTime();

	GTSL::StaticString<maxLogLength> string;
	
	char buffer[1024];

	const uint32 date_length = snprintf(buffer, 1024, "Counter: %u, Thread: %u, [Date: %02d/%02d/%02d]", counter++, GTSL::Thread::ThisTreadID(), day_of_month, month, year);
	string += buffer;

	const uint32 time_length = snprintf(buffer, 1024, "[Time: %02d:%02d:%02d]", time.Hour, time.Minute, time.Second);
	string += buffer;

	const uint32 text_chars_to_write = text.Bytes() + 2 > string.GetCapacity() - string.GetLength() ? string.GetLength() - 1 : text.Bytes();

	string += GTSL::Ranger<const UTF8>(text_chars_to_write, text.begin()); string += '\n';
	
	if(verbosityLevel >= minLogLevel)
	{
		SetTextColorOnLogLevel(verbosityLevel);
		printf(string.begin());
	}

	logMutex.Lock();
	logFile.WriteToFile(GTSL::Ranger<const byte>(string.GetLength(), reinterpret_cast<const byte*>(string.begin())));
	logMutex.Unlock();
}

void Logger::logFunctionTimer(FunctionTimer* functionTimer, GTSL::Microseconds timeTaken)
{
	//log(VerbosityLevel::MESSAGE, GTSL::Ranger<UTF8>(GTSL::StringLength(functionTimer->Name), functionTimer->Name));
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