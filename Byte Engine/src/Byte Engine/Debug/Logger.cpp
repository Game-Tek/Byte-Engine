#include "Logger.h"

#include "Byte Engine/Application/Application.h"
#include "Byte Engine/Application/Clock.h"

#include <cstdio>
#define WIN32_LEAN_AND_MEAN
#include <Windows.h>

using namespace BE;

BE::PersistentAllocatorReference allocator_reference("Clock", true); //TODO get rid of global once vector is fixed

Logger::Logger(const LoggerCreateInfo& loggerCreateInfo) : logFile(), fileBuffer(defaultBufferLength * 2/*use single buffer as two buffers for when one half is being written to disk*/, &allocator_reference)
{
	logFile.OpenFile(loggerCreateInfo.AbsolutePathToLogFile, GTSL::File::OpenFileMode::WRITE);
}

void Logger::Shutdown() const
{
	uint64 bytes_written{ 0 };
	logMutex.ReadLock();
	logFile.WriteToFile(GTSL::Ranger<byte>(reinterpret_cast<byte*>(fileBuffer.begin()), reinterpret_cast<byte*>(fileBuffer.begin() + fileBuffer.GetLength())), bytes_written);
	logMutex.ReadUnlock();
	logFile.CloseFile();
}

void Logger::PrintObjectLog(const Object* obj, const VerbosityLevel level, const char* text, ...) const
{
	const auto day_of_month = Clock::GetDayOfMonth(); const auto month = Clock::GetMonth(); const auto year = Clock::GetYear(); const auto time = Clock::GetTime();
	
	auto print_date_to_buffer = [&]() { return snprintf(const_cast<char*>(fileBuffer.begin() + fileBuffer.GetLength()), fileBuffer.GetCapacity(), "[Date: %02d/%02hhu/%02d]", day_of_month, month, year); };
	auto print_log_to_buffer = [&]() { return snprintf(fileBuffer.GetData(), fileBuffer.GetCapacity() - fileBuffer.GetLength(), "[Time: %02d:%02d:%02d] %s: %s", time.Hour, time.Minute, time.Second, obj->GetName(), text); };

	// Check if should dump logs to file if no more space is available
	logMutex.ReadLock();
	auto date_text_length = print_date_to_buffer() + 1;
	if (date_text_length > fileBuffer.GetRemainingLength())
	{
		uint64 bytes_written{ 0 };
		//TODO dispatch as a job
		logFile.WriteToFile(GTSL::Ranger<byte>((byte*)fileBuffer.begin() + currentBufferStart, (byte*)fileBuffer.begin() + currentBufferStart + fileBuffer.GetLength()), bytes_written);
		logMutex.ReadUnlock();
		
		{
			logMutex.WriteLock();
			currentBufferStart = currentBufferStart == 0 ? fileBuffer.GetCapacity() : 0;
			fileBuffer.Resize(0);
			logMutex.WriteUnlock();
		}
	}
	else
	{
		logMutex.ReadUnlock();
	}

	// Print to buffer now that we are sure space is available
	logMutex.ReadLock();
	date_text_length = print_date_to_buffer() + 1;
	logMutex.ReadUnlock();
	{
		logMutex.WriteLock();
		fileBuffer.Resize(fileBuffer.GetLength() + date_text_length);
		logMutex.WriteUnlock();
	}
		
	// Print log to buffer if no space to hold single log resize buffer
	logMutex.ReadLock();
	auto log_length = print_log_to_buffer() + 1;
	if(log_length > fileBuffer.GetRemainingLength())
	{
		logMutex.ReadUnlock();
		//TODO: WHAT IF FILE BUFFER IS NOT BIG ENOUGH TO HOLD LOG, RESIZE?, ASSERT?(ASSERT WILL CALL LOG FOR WHICH THERE WILL BE NO SPACE!)
		logMutex.WriteLock();
		fileBuffer.Resize(fileBuffer.GetCapacity() + log_length);
		logMutex.WriteUnlock();
	}
	else
	{
		logMutex.ReadUnlock();
	}

	// Print log to buffer now that we are sure space is available
	logMutex.ReadLock();
	log_length = print_log_to_buffer() + 1;
	logMutex.ReadUnlock();
	
	logMutex.WriteLock();
	fileBuffer.Resize(fileBuffer.GetLength() + log_length);
	logMutex.WriteUnlock();
	
	if (level >= minLogLevel)
	{
		SetTextColorOnLogLevel(level);
		
		logMutex.ReadLock();
		printf(fileBuffer.begin() + currentBufferStart + fileBuffer.GetLength() + date_text_length); //todo
		logMutex.ReadUnlock();
	}
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
	if (level >= minLogLevel)
	{
		SetTextColorOnLogLevel(level);

		const Time LogTime = Clock::GetTime();

		//printf();

		va_list args;
		va_start(args, text);
		vprintf(text, args);
		va_end(args);

		printf("\n");
	}
}
