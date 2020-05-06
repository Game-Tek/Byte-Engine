#pragma once

#include <GTSL/Mutex.h>
#include <GTSL/File.h>
#include <GTSL/Vector.hpp>

#include "Byte Engine/Core.h"

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
		mutable GTSL::ReadWriteMutex logMutex;
		
		/**
		 * \brief Minimum level for a log to go through to console, all logs get dumped to disk.
		 */
		mutable VerbosityLevel minLogLevel;

		/**
		 * \brief File handle to log file where all logs are dumped to.
		 */
		mutable GTSL::File logFile;

		/**
		 * \brief Default amount of characters the buffer can hold at a moment.
		 */
		static constexpr uint32 defaultBufferLength{ 10000 };
		
		/**
		 * \brief Current write index in the buffer, this is swapped every time the memory buffer is dumped to a file since we use a single buffer as two to avoid contention.
		 */
		mutable uint32 currentBufferStart{ 0 };
		
		mutable GTSL::Vector<char> fileBuffer;

		void SetTextColorOnLogLevel(VerbosityLevel level) const;
		void log(VerbosityLevel verbosityLevel, const GTSL::Ranger<char>& text) const;
	public:
		Logger() = default;
		~Logger();

		struct LoggerCreateInfo
		{
			GTSL::Ranger<char> AbsolutePathToLogFile;
		};
		explicit Logger(const LoggerCreateInfo& loggerCreateInfo);

		void Shutdown() const;

		void PrintObjectLog(const Object* obj, VerbosityLevel level, const char* text, ...) const;
		void PrintBasicLog(VerbosityLevel level, const char* text, ...) const;

		/**
		 * \brief Sets the minimum log verbosity, only affects logs to console. Value is inclusive.
		 * \param level Verbosity level.
		 */
		void SetMinLogLevel(const VerbosityLevel level) const
		{
			GTSL::WriteLock<GTSL::ReadWriteMutex> lock(logMutex);
			minLogLevel = level;
		}
	};
}