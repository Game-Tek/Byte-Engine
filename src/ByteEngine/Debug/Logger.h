#pragma once

#include <atomic>
#include <GTSL/Mutex.h>
#include <GTSL/File.hpp>
#include <GTSL/String.hpp>
#include <GTSL/Time.h>
#include "ByteEngine/Id.h"
#include "ByteEngine/Object.h"
#include "ByteEngine/Core.h"

class FunctionTimer;
class Object;

#undef ERROR

namespace BE
{
	constexpr const char8_t* FIX_OR_CRASH_STRING = u8"Fix this issue as it will lead to a crash";

	/**
	 * \brief Self locking class that manages logging to console and to disk.
	 * All logs get dumped to disk, verbosity levels are only for console.
	 */
	class Logger : public Object
	{
	public:
		enum class VerbosityLevel : GTSL::uint8
		{
			MESSAGE = 1,
			SUCCESS = 2,
			WARNING = 4,
			FATAL = 8
		};

		Logger() = default;
		~Logger();

		struct LoggerCreateInfo
		{
			GTSL::Range<const char8_t*> LogDirAbsolutePath;
		};

		explicit Logger(const LoggerCreateInfo* createInfo);

		void SetTrace(bool trace);

		template<typename... Args>
		void PrintObjectLog(const Object* obj, const VerbosityLevel level, Args... args)
		{
			GTSL::StaticString<m_maxLogLength> msg;
			msg += obj->GetName();
			msg += u8": ";
			(GTSL::ToString(msg, GTSL::ForwardRef<Args>(args)), ...);
			log(level, msg);
		}

		template<typename... Args>
		void PrintBasicLog(const VerbosityLevel level, Args&& ...args)
		{
			GTSL::StaticString<m_maxLogLength> msg;
			(GTSL::ToString(msg, GTSL::ForwardRef<Args>(args)), ...);
			log(level, msg);
		}

		/**
		 * \brief Sets the minimum log verbosity, only affects logs to console. Value is inclusive.
		 * \param level Verbosity level.
		 */
		void SetMinLogLevel(const VerbosityLevel level) const
		{
			m_logMutex.Lock();
			m_minLogLevel = level;
			m_logMutex.Unlock();
		}

		void logFunction(const GTSL::StringView name, GTSL::Microseconds startTime, GTSL::Microseconds endTime, const GTSL::StringView args)
		{
			if (!m_trace) { return; }

			GTSL::StaticString<1024> string;

			{
				GTSL::Lock lock(m_traceMutex);

				if (m_profileCount++ > 0)
					string += u8",";

				string += u8"{";
				string += u8"\"cat\":\"function\",";
				string += u8"\"dur\":"; ToString(string, (endTime - startTime).GetCount()); string += u8",";
				string += u8"\"name\":\""; string += name; string += u8"\",";
				string += u8"\"ph\":\"X\",";
				string += u8"\"pid\":0,";
				string += u8"\"tid\":"; ToString(string, GetThread()); string += u8",";
				string += u8"\"ts\":"; ToString(string, startTime.GetCount());
				if (args.GetBytes()) {
					string += u8','; string += u8"\"args\":{ "; string += args; string += u8"}";
				}
				string += u8"}]}";

				m_graphFile.SetPointer(m_graphFile.GetSize() - 2);
				m_graphFile.Write(GTSL::Range<const GTSL::uint8*>(string.GetBytes(), reinterpret_cast<const GTSL::uint8*>(string.c_str())));
			}
		}

		void InstantEvent(GTSL::StringView name, GTSL::uint64 time) {
			if (!m_trace) { return; }

			GTSL::StaticString<1024> string;

			{
				GTSL::Lock lock(m_traceMutex);

				if (m_profileCount++ > 0)
					string += u8",";

				string += u8"{";
				string += u8"\"name\":\""; string += name; string += u8"\",";
				string += u8"\"ph\":\"i\",";
				string += u8"\"ts\":"; ToString(string, time); string += u8",";
				string += u8"\"pid\":0,";
				string += u8"\"tid\":"; ToString(string, GetThread()); string += u8",";
				string += u8"\"s\":\"g\"";
				string += u8"}";

				m_graphFile.Write(GTSL::Range(string.GetBytes(), reinterpret_cast<const GTSL::uint8*>(string.c_str())));
			}
		}
	private:
		/**
		 * \brief Mutex for all log operations.
		 */
		mutable GTSL::Mutex m_logMutex;
		mutable GTSL::Mutex m_traceMutex;

		/**
		 * \brief Minimum level for a log to go through to console, all logs get dumped to disk.
		 */
		mutable VerbosityLevel m_minLogLevel{ VerbosityLevel::MESSAGE };

		/**
		 * \brief File handle to log file where all logs are dumped to.
		 */
		mutable GTSL::File m_logFile;
		mutable GTSL::File m_graphFile;
		mutable GTSL::uint64 m_profileCount = 0;

		static constexpr GTSL::uint16 m_maxLogLength{ 8192 };

		static constexpr GTSL::uint32 m_bytesToDumpOn{ 256 };

		/**
		 * \brief Default amount of characters the buffer can hold at a moment.
		 */
		static constexpr GTSL::uint32 m_defaultBufferLength{ m_bytesToDumpOn };

		mutable std::atomic<GTSL::uint32> m_posInBuffer{ 0 };

		mutable char8_t* m_data{ nullptr };
		bool m_trace;

		mutable std::atomic<GTSL::uint32> m_counter{0};

		void SetTextColor(VerbosityLevel level) const;
		void log(VerbosityLevel level, const GTSL::Range<const char8_t*> msg) const;
	};
}