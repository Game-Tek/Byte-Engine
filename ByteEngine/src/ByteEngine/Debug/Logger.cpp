#include "Logger.h"

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/Clock.h"
#include "ByteEngine/Debug/FunctionTimer.h"

using namespace BE;

static SystemAllocatorReference allocator_reference("Logger", true); //TODO get rid of global once vector is fixed

Logger::Logger(const LoggerCreateInfo& loggerCreateInfo) : logFile()
{
	uint64 allocated_size{ 0 };
	allocator_reference.Allocate(defaultBufferLength, 1, reinterpret_cast<void**>(&data), &allocated_size);
	
	GTSL::Memory::SetZero(defaultBufferLength, data);
	
	GTSL::StaticString<1024> path(loggerCreateInfo.AbsolutePathToLogDirectory);
	path += "/log.txt";
	logFile.OpenFile(path, GTSL::File::OpenFileMode::WRITE);
}

void Logger::Shutdown() const
{
	uint64 bytes_written{ 0 };
	logMutex.Lock();
	logFile.WriteToFile(GTSL::Ranger<byte>(reinterpret_cast<byte*>(data) + buffersInBuffer * subBufferIndex, reinterpret_cast<byte*>(data + posInSubBuffer + buffersInBuffer * subBufferIndex)), bytes_written);
	logMutex.Unlock();
	logFile.CloseFile();
}

void Logger::log(const VerbosityLevel verbosityLevel, const GTSL::Ranger<GTSL::UTF8>& text) const
{
	const auto day_of_month = Clock::GetDayOfMonth(); const auto month = Clock::GetMonth(); const auto year = Clock::GetYear(); const auto time = Clock::GetTime();

	char string[logMaxLength]{ 0 };
	
	uint32 string_remaining_length{ logMaxLength };
	uint32 write_start{ posInSubBuffer };
	uint32 written_bytes{ 0 };

	auto write_loc = [&]() {return written_bytes; };
	auto write_ptr = [&]() { return string + write_loc(); };

	const uint32 date_length = snprintf(write_ptr(), string_remaining_length, "[Date: %02d/%02hhu/%02d]", day_of_month, month, year);
	string_remaining_length -= date_length;
	written_bytes += date_length;

	const uint32 time_length = snprintf(write_ptr(), string_remaining_length, "[Time: %02d:%02d:%02d]", time.Hour, time.Minute, time.Second);
	string_remaining_length -= time_length;
	written_bytes += time_length;

	const uint32 text_length = text.ElementCount(); //TODO: ASSERT LENGTH
	{
		const uint32 bytes_to_copy = text_length + 2 > string_remaining_length ? string_remaining_length - 2 : text_length;
		string_remaining_length -= bytes_to_copy;
		GTSL::Memory::MemCopy(bytes_to_copy, text.begin(), write_ptr());
		written_bytes += bytes_to_copy;
	}

	written_bytes += 1; //null terminator
	
	string[write_loc() - 1] = '\n';
	string[write_loc()] = '\0';
	
	if(verbosityLevel >= minLogLevel)
	{
		SetTextColorOnLogLevel(verbosityLevel);
		printf(string + date_length);
	}
	
	// Check if should dump logs to file if no more space is available
	if (posInSubBuffer + written_bytes >= bytesToDumpOn)
	{
		if(write_start >= defaultBufferLength)
		{
		}
		
		uint64 bytes_written{ 0 };
		//TODO dispatch as a job
		logMutex.Lock();
		logFile.WriteToFile(GTSL::Ranger<byte>(reinterpret_cast<byte*>(data) + subBufferIndex * buffersInBuffer, reinterpret_cast<byte*>(data) + posInSubBuffer + subBufferIndex * buffersInBuffer), bytes_written);
		logMutex.Unlock();
		
		posInSubBuffer = 0;
		subBufferIndex = (subBufferIndex + 1) % buffersInBuffer;
	}

	logMutex.Lock();
	GTSL::Memory::MemCopy(written_bytes, string, data + posInSubBuffer + subBufferIndex * buffersInBuffer);
	logMutex.Unlock();

	posInSubBuffer += written_bytes;
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