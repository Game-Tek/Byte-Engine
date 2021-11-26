#pragma once

#include <atomic>
#include <GTSL/Mutex.h>
#include <GTSL/File.h>

#include "ByteEngine/Core.h"
#include <GTSL/String.hpp>
#include <GTSL/Time.h>

#include "ByteEngine/Id.h"
#include "ByteEngine/Object.h"

class FunctionTimer;
class Object;

#undef ERROR

namespace BE
{
	constexpr const utf8* FIX_OR_CRASH_STRING = u8"Fix this issue as it will lead to a crash in release mode!";
	
	/**
	 * \brief Self locking class that manages logging to console and to disk.
	 * All logs get dumped to disk, verbosity levels are only for console.
	 */
	class Logger : public Object
	{
	public:
		enum class VerbosityLevel : uint8 {
			MESSAGE = 1, SUCCESS = 2, WARNING = 4, FATAL = 8
		};
	private:
		/**
		 * \brief Mutex for all log operations.
		 */
		mutable GTSL::Mutex logMutex;
		mutable GTSL::Mutex traceMutex;
		
		/**
		 * \brief Minimum level for a log to go through to console, all logs get dumped to disk.
		 */
		mutable VerbosityLevel minLogLevel{ VerbosityLevel::MESSAGE };

		/**
		 * \brief File handle to log file where all logs are dumped to.
		 */
		mutable GTSL::File logFile;
		mutable GTSL::File graphFile;
		mutable uint64 profileCount = 0;

		static constexpr uint16 maxLogLength{ 8192 };

		static constexpr uint32 bytesToDumpOn{ 256 };
		
		/**
		 * \brief Default amount of characters the buffer can hold at a moment.
		 */
		static constexpr uint32 defaultBufferLength{ bytesToDumpOn };

		mutable std::atomic<uint32> posInBuffer{ 0 };

		//mutable GTSL::HashMap<Id, Id, BE::SystemAllocatorReference> allowedLoggers;
		
		mutable utf8* data{ nullptr };

		bool trace = false;

		mutable std::atomic<uint32> counter{ 0 };
		
		void SetTextColorOnLogLevel(VerbosityLevel level) const;
		void log(VerbosityLevel verbosityLevel, const GTSL::Range<const char8_t*> text) const;

		friend class FunctionTimer;
		void logFunctionTimer(FunctionTimer* functionTimer, GTSL::Microseconds timeTaken);
	public:
		Logger() = default;
		~Logger();

		struct LoggerCreateInfo {
			GTSL::Range<const utf8*> AbsolutePathToLogDirectory;
			bool Trace = false;
		};
		explicit Logger(const LoggerCreateInfo& loggerCreateInfo);

		template<typename... ARGS>
		void PrintObjectLog(const Object* obj, const VerbosityLevel level, ARGS... args) {
			GTSL::StaticString<maxLogLength> text;
			text += obj->GetName(); text += u8": ";
			(ToString(text, GTSL::ForwardRef<ARGS>(args)), ...);
			log(level, text);
		}

		template<typename... ARGS>
		void PrintBasicLog(const VerbosityLevel level, ARGS&& ...args) {
			GTSL::StaticString<maxLogLength> text;
			//(ToString(text, GTSL::ForwardRef<ARGS>(args)), ...);
			log(level, text);
		}
		
		/**
		 * \brief Sets the minimum log verbosity, only affects logs to console. Value is inclusive.
		 * \param level Verbosity level.
		 */
		void SetMinLogLevel(const VerbosityLevel level) const {
			logMutex.Lock();
			minLogLevel = level;
			logMutex.Unlock();
		}

		void InstantEvent(GTSL::StringView name, uint64 time) {
			if (!trace) { return; }

			GTSL::StaticString<1024> string;

			{
				GTSL::Lock lock(traceMutex);

				if (profileCount++ > 0)
					string += u8",";

				string += u8"{";
				string += u8"\"name\":\""; string += name; string += u8"\",";
				string += u8"\"ph\":\"i\",";
				string += u8"\"ts\":"; ToString(string, time); string += u8",";
				string += u8"\"pid\":0,";
				string += u8"\"tid\":"; ToString(string, getThread()); string += u8",";
				string += u8"\"s\":\"g\"";
				string += u8"}";

				graphFile.Write(GTSL::Range(string.GetBytes(), reinterpret_cast<const byte*>(string.c_str())));
			}
		}
	};
}