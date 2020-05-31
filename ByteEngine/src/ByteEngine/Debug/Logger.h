#pragma once

#include <GTSL/Mutex.h>
#include <GTSL/File.h>
#include <GTSL/Vector.hpp>

#include "ByteEngine/Core.h"

#include <GTSL/StaticString.hpp>
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

		static constexpr uint32 logMaxLength{ 1024 };
		
		static constexpr uint32 bytesToDumpOn{ 256 };
		
		/**
		 * \brief Default amount of characters the buffer can hold at a moment.
		 */
		static constexpr uint32 defaultBufferLength{ bytesToDumpOn * 3 };

		
		/**
		 * \brief Current write index in the buffer, this is swapped every time the memory buffer is dumped to a file since we use a single buffer as two to avoid contention.
		 */
		mutable std::atomic<uint32> currentStringIndex{ 0 };
		
		mutable GTSL::Vector<char> fileBuffer;

		mutable std::atomic<uint32> lastWriteToDiskPos{ 0 };
		mutable std::atomic<uint32> bytesWrittenSinceLastWriteToDisk{ 0 };

		void SetTextColorOnLogLevel(VerbosityLevel level) const;
		void log(VerbosityLevel verbosityLevel, const GTSL::Ranger<char>& text) const;
	public:
		Logger() = default;
		~Logger();

		struct LoggerCreateInfo
		{
			GTSL::Ranger<UTF8> AbsolutePathToLogDirectory;
		};
		explicit Logger(const LoggerCreateInfo& loggerCreateInfo);

		void Shutdown() const;

		void PrintObjectLog(const Object* obj, VerbosityLevel level, const char* text, ...) const;
		void PrintBasicLog(VerbosityLevel level, const char* text, ...) const;

		template<typename... ARGS>
		void PrintBLog(const VerbosityLevel level, ARGS& ...args)
		{
			GTSL::StaticString<1024> text;
			(text += ... += args);
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