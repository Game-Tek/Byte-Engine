#pragma once

#include <GTSL/Mutex.h>
#include <GTSL/File.h>
#include <GTSL/Console.h>

#include "ByteEngine/Core.h"
#include <GTSL/StaticString.hpp>
#include <GTSL/Time.h>

#include "ByteEngine/Object.h"

class FunctionTimer;
class Object;

#undef ERROR

namespace BE
{
	/**
	 * \brief Self locking class that manages logging to console and to disk.
	 * All logs get dumped to disk, verbosity levels are only for console.
	 */
	class Logger
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
		mutable VerbosityLevel minLogLevel;

		/**
		 * \brief File handle to log file where all logs are dumped to.
		 */
		mutable GTSL::File logFile;

		static constexpr uint32 bytesToDumpOn{ 256 };
		
		static constexpr uint32 logMaxLength{ bytesToDumpOn };

		static constexpr uint32 buffersInBuffer{ 3 };
		
		/**
		 * \brief Default amount of characters the buffer can hold at a moment.
		 */
		static constexpr uint32 defaultBufferLength{ bytesToDumpOn * buffersInBuffer };

		mutable std::atomic<uint32> posInSubBuffer{ 0 };
		mutable std::atomic<uint32> subBufferIndex{ 0 };
		
		mutable UTF8* data{ nullptr };

		GTSL::Console console;
		
		void SetTextColorOnLogLevel(VerbosityLevel level) const;
		void log(VerbosityLevel verbosityLevel, const GTSL::Ranger<char>& text) const;

		friend class FunctionTimer;
		void logFunctionTimer(FunctionTimer* functionTimer, GTSL::Microseconds timeTaken);
	public:
		Logger() = default;
		~Logger();

		struct LoggerCreateInfo
		{
			GTSL::Ranger<const UTF8> AbsolutePathToLogDirectory;
		};
		explicit Logger(const LoggerCreateInfo& loggerCreateInfo);

		void Shutdown() const;

		template<typename... ARGS>
		void PrintObjectLog(const Object* obj, const VerbosityLevel level, ARGS&& ...args)
		{
			GTSL::StaticString<1024> text;
			text += obj->GetName(); text += ": ";
			(text += ... += GTSL::MakeForwardReference<ARGS>(args));
			log(level, text);
		}

		template<typename... ARGS>
		void PrintBasicLog(const VerbosityLevel level, ARGS&& ...args)
		{
			GTSL::StaticString<1024> text;
			(text += ... += GTSL::MakeForwardReference<ARGS>(args));
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