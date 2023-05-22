#include "Logger.h"

#include "ByteEngine/Application/Clock.h"
#include "ByteEngine/Debug/FunctionTimer.h"
#include "ByteEngine/Application/Application.h"

#include <GTSL/System.hpp>

#include "ByteEngine/Application/AllocatorReferences.h"

namespace BE
{
	Logger::Logger(const LoggerCreateInfo* createInfo)
		: Object(u8"Logger")
	{
		GTSL::uint64* allocated_size{nullptr};

		GetPersistentAllocator().Allocate(m_defaultBufferLength, 1, reinterpret_cast<void**>(&m_data), &allocated_size);

		{
			GTSL::StaticString<260> path(createInfo->LogDirAbsolutePath);
			path += u8"/log.txt";
			switch(m_logFile.Open(path,GTSL::File::WRITE,true))
			{
			case GTSL::File::OpenResult::OK: break;
			case GTSL::File::OpenResult::CREATED: break;
			case GTSL::File::OpenResult::ERROR: break;
			default:;
			}
		}
		m_logFile.Resize(0);
		GTSL::Console::SetConsoleInputModeAsUTF8();
	}

	void Logger::SetTrace(bool trace)
	{
		m_trace = trace;

		if(m_trace)
		{
			GTSL::StaticString<260> path(BE::Application::Get()->GetPathToApplication());
			path += u8"/trace.txt";
			switch (m_graphFile.Open(path, GTSL::File::WRITE, true))
			{
			case GTSL::File::OpenResult::OK: break;
			case GTSL::File::OpenResult::CREATED: break;
			case GTSL::File::OpenResult::ERROR: break;
			default:;
			}

			m_graphFile.Resize(0);

			GTSL::StaticString<512> string;
			string += u8"{\"otherData\": {},\"traceEvents\":[]}";
			m_graphFile.Write(GTSL::Range(string.GetBytes(), reinterpret_cast<const GTSL::uint8*>(string.c_str())));
		}
	}

	//TODO: if string is too big clamp to nearest CODEPOINT not Byte
	void Logger::log(VerbosityLevel level, const GTSL::Range<const char8_t*> msg) const
	{
		const auto day = Clock::GetDay();
		const auto month = Clock::GetMonth();
		const auto year = Clock::GetYear();
		const auto time = Clock::GetTime();

		GTSL::StaticString<m_maxLogLength> string;

		string += u8"[";
		GTSL::ToString(string, day);
		string += u8"/";
		GTSL::ToString(string, static_cast<GTSL::uint8>(month));
		string += u8"/";
		GTSL::ToString(string, year);
		string += u8"]";

		string += u8"[";
		GTSL::ToString(string, time.Hour);
		string += u8"/";
		GTSL::ToString(string, static_cast<GTSL::uint8>(time.Minute));
		string += u8"/";
		GTSL::ToString(string, time.Second);
		string += u8"]";

		auto clampedSize = GTSL::Math::Limit(msg.GetBytes(), string.GetCapacity() - string.GetBytes() - 1);
		string += GTSL::Range<const char8_t*>(clampedSize, clampedSize, msg.GetData());

		string += u8'\n';

		if(level >= m_minLogLevel)
		{
			SetTextColor(level);
			GTSL::Console::Print(string);
		}

		m_logMutex.Lock();
		m_logFile.Write(GTSL::Range<const GTSL::uint8*>(string.GetBytes(), reinterpret_cast<const GTSL::uint8*>(string.c_str())));
		m_logMutex.Unlock();
	}

	void Logger::SetTextColor(VerbosityLevel level) const
	{
		switch(level)
		{
		case VerbosityLevel::MESSAGE: GTSL::Console::SetTextColor(GTSL::Console::TextColor::WHITE); break;
		case VerbosityLevel::SUCCESS: GTSL::Console::SetTextColor(GTSL::Console::TextColor::GREEN);  break;
		case VerbosityLevel::WARNING: GTSL::Console::SetTextColor(GTSL::Console::TextColor::ORANGE);  break;
		case VerbosityLevel::FATAL: GTSL::Console::SetTextColor(GTSL::Console::TextColor::RED);  break;
		default: GTSL::Console::SetTextColor(GTSL::Console::TextColor::WHITE);
		}
	}

	Logger::~Logger()
	{
		
	}

}
