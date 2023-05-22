#pragma once

#include <GTSL/StringCommon.h>
#include <GTSL/ShortString.hpp>

#include "Application/AllocatorReferences.h"

namespace BE
{
	class Logger;
}

/**
 * \brief Base class for most non-data only classes in the engine.
 */
class Object
{
public:
	Object() = default;
	Object(const GTSL::StringView name) : m_objectName(name) {}

	~Object() = default;

	[[nodiscard]] const GTSL::ShortString<128>& GetName() const { return m_objectName; }

	[[nodiscard]] BE::PersistentAllocatorReference GetPersistentAllocator() const
	{
		return {GetName()};
	}

	[[nodiscard]] BE::TransientAllocatorReference GetTransientAllocator() const
	{
		return {GetName()};
	}

protected:
	[[nodiscard]] BE::Logger* GetLogger() const;
	[[nodiscard]] GTSL::uint8 GetThread() const;
private:
	GTSL::ShortString<128> m_objectName{u8"Object"};
};

#ifdef BE_DEBUG
#define BE_LOG_SUCCESS(...)		this->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::SUCCESS, __VA_ARGS__);
#define BE_LOG_MESSAGE(...)		this->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::MESSAGE, __VA_ARGS__);
#define BE_LOG_WARNING(...)		this->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::WARNING, __VA_ARGS__);
#define BE_LOG_ERROR(...)		this->GetLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::FATAL, __VA_ARGS__);
#define BE_LOG_LEVEL(Level)		this->GetLogger()->SetMinLogLevel(Level);
#else
#define BE_LOG_SUCCESS(Text, ...)
#define BE_LOG_MESSAGE(Text, ...)
#define BE_LOG_WARNING(Text, ...)
#define BE_LOG_ERROR(Text, ...)
#define BE_LOG_LEVEL(_Level)
#define BE_BASIC_LOG_SUCCESS(Text, ...)	
#define BE_BASIC_LOG_MESSAGE(Text, ...)	
#define BE_BASIC_LOG_WARNING(Text, ...)	
#define BE_BASIC_LOG_ERROR(Text, ...)	
#endif