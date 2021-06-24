#pragma once

#include <atomic>
#include <GTSL/Mutex.h>
#include <GTSL/File.h>
#include <GTSL/Console.h>

#include "ByteEngine/Core.h"
#include <GTSL/StaticString.hpp>
#include <GTSL/Time.h>
#include <GTSL/HashMap.h>


#include "ByteEngine/Id.h"
#include "ByteEngine/Object.h"

class FunctionTimer;
class Object;

#undef ERROR

namespace BE
{
	constexpr const char* FIX_OR_CRASH_STRING = "Fix this issue as it will lead to a crash in release mode!";
	
	/**
	 * \brief Self locking class that manages logging to console and to disk.
	 * All logs get dumped to disk, verbosity levels are only for console.
	 */
	class Logger : public Object
	{
	public:
		enum class VerbosityLevel : uint8
		{
			MESSAGE = 1, SUCCESS = 2, WARNING = 4, FATAL = 8
		};
	private:
		/**
		 * \brief Mutex for all log operations.
		 */
		mutable GTSL::Mutex logMutex;
		
		/**
		 * \brief Minimum level for a log to go through to console, all logs get dumped to disk.
		 */
		mutable VerbosityLevel minLogLevel{ VerbosityLevel::MESSAGE };

		/**
		 * \brief File handle to log file where all logs are dumped to.
		 */
		mutable GTSL::File logFile;

		static constexpr uint16 maxLogLength{ 8192 };

		static constexpr uint32 bytesToDumpOn{ 256 };
		
		/**
		 * \brief Default amount of characters the buffer can hold at a moment.
		 */
		static constexpr uint32 defaultBufferLength{ bytesToDumpOn };

		mutable std::atomic<uint32> posInBuffer{ 0 };

		mutable GTSL::HashMap<Id, Id, BE::SystemAllocatorReference> allowedLoggers;
		
		mutable utf8* data{ nullptr };

		mutable std::atomic<uint32> counter{ 0 };
		
		void SetTextColorOnLogLevel(VerbosityLevel level) const;
		void log(VerbosityLevel verbosityLevel, const GTSL::Range<const char*> text) const;

		friend class FunctionTimer;
		void logFunctionTimer(FunctionTimer* functionTimer, GTSL::Microseconds timeTaken);
	public:
		Logger() = default;
		~Logger();

		struct LoggerCreateInfo
		{
			GTSL::Range<const utf8*> AbsolutePathToLogDirectory;
		};
		explicit Logger(const LoggerCreateInfo& loggerCreateInfo);

		template<typename... ARGS>
		void PrintObjectLog(const Object* obj, const VerbosityLevel level, ARGS... args)
		{
			GTSL::StaticString<maxLogLength> text;
			text += obj->GetName(); text += ": ";
			(text += ... += GTSL::ForwardRef<ARGS>(args));
			log(level, text);
		}

		template<typename... ARGS>
		void PrintBasicLog(const VerbosityLevel level, ARGS&& ...args)
		{
			GTSL::StaticString<maxLogLength> text;
			(text += ... += GTSL::ForwardRef<ARGS>(args));
			log(level, text);
		}
		
		/**
		 * \brief Sets the minimum log verbosity, only affects logs to console. Value is inclusive.
		 * \param level Verbosity level.
		 */
		void SetMinLogLevel(const VerbosityLevel level) const
		{
			logMutex.Lock();
			minLogLevel = level;
			logMutex.Unlock();
		}
	};
}