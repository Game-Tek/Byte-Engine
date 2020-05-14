#include "Logger.h"

#include "Byte Engine/Application/Application.h"
#include "Byte Engine/Application/Clock.h"

#include <cstdio>
#define WIN32_LEAN_AND_MEAN
#include <Windows.h>
#include <GTSL/Array.hpp>
#include <GTSL\StaticString.hpp>

using namespace BE;

BE::PersistentAllocatorReference allocator_reference("Clock", true); //TODO get rid of global once vector is fixed

Logger::Logger(const LoggerCreateInfo& loggerCreateInfo) : logFile(), fileBuffer(defaultBufferLength * 2/*use single buffer as two buffers for when one half is being written to disk*/, &allocator_reference)
{
	GTSL::StaticString<1024> path(loggerCreateInfo.AbsolutePathToLogFile);
	path += "/log.txt";
	fileBuffer.Resize(fileBuffer.GetCapacity());
	logFile.OpenFile(path, GTSL::File::OpenFileMode::WRITE);
}

void Logger::Shutdown() const
{
	uint64 bytes_written{ 0 };
	logMutex.ReadLock();
	logFile.WriteToFile(GTSL::Ranger<byte>(reinterpret_cast<byte*>(fileBuffer.begin()), reinterpret_cast<byte*>(fileBuffer.begin() + fileBuffer.GetLength())), bytes_written);
	logMutex.ReadUnlock();
	logFile.CloseFile();
}

void Logger::log(const VerbosityLevel verbosityLevel, const GTSL::Ranger<GTSL::UTF8>& text) const
{
	const auto day_of_month = Clock::GetDayOfMonth(); const auto month = Clock::GetMonth(); const auto year = Clock::GetYear(); const auto time = Clock::GetTime();

	uint32 string_remaining_length{ perStringMaxLength };
	uint32 write_start{ currentStringIndex * perStringMaxLength };
	
	// Check if should dump logs to file if no more space is available
	if (write_start >= defaultBufferLength)
	{
		uint64 bytes_written{ 0 };
		//TODO dispatch as a job
		logFile.WriteToFile(GTSL::Ranger<byte>((byte*)fileBuffer.begin() + write_start, (byte*)fileBuffer.begin() + write_start + (fileBuffer.GetLength() - write_start)), bytes_written);
	}

	uint32 date_length = snprintf(const_cast<char*>(fileBuffer.begin() + write_start), string_remaining_length, "[Date: %02d/%02hhu/%02d]", day_of_month, month, year);
	string_remaining_length -= date_length;
	write_start += date_length;
	
	uint32 time_length = snprintf(fileBuffer.GetData() + write_start, string_remaining_length, "[Time: %02d:%02d:%02d]", time.Hour, time.Minute, time.Second);
	string_remaining_length -= time_length;
	write_start += time_length;
	
	uint32 text_length = text.ElementCount(); //TODO: ASSERT LENGTH
	string_remaining_length -= text_length;
	GTSL::Memory::MemCopy(text_length, text.Data(), fileBuffer.GetData() + write_start);
	write_start += text_length;
	
	fileBuffer[write_start - 1] = '\n';
	fileBuffer[write_start] = '\0';

	if(verbosityLevel >= minLogLevel)
	{
		SetTextColorOnLogLevel(verbosityLevel);
		printf(fileBuffer.GetData() + currentStringIndex * perStringMaxLength + date_length);
	}

	++currentStringIndex;
}

void Logger::PrintObjectLog(const Object* obj, const VerbosityLevel level, const char* text, ...) const
{
	GTSL::StaticString<1024> t;
	t += obj->GetName(); t += ": "; t += text;
	log(level, t);
}

void Logger::SetTextColorOnLogLevel(const VerbosityLevel level) const
{
	switch (level)
	{
	default: SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15); break;

	case VerbosityLevel::MESSAGE: SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), 15); break;

	case VerbosityLevel::SUCCESS: SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), FOREGROUND_GREEN | FOREGROUND_INTENSITY); break;

	case VerbosityLevel::WARNING: SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), FOREGROUND_RED | FOREGROUND_GREEN | FOREGROUND_INTENSITY); break;
#undef ERROR
	case VerbosityLevel::FATAL: SetConsoleTextAttribute(GetStdHandle(STD_OUTPUT_HANDLE), FOREGROUND_RED | FOREGROUND_INTENSITY); break;
	}
}

Logger::~Logger()
{
}

void Logger::PrintBasicLog(const VerbosityLevel level, const char* text, ...) const
{
	GTSL::StaticString<1024> t;
	t += text;
	log(level, t);
}
