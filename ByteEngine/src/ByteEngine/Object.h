#pragma once

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
	
	Object(const utf8* objectName) : name(objectName) {}
	
	~Object() = default;

	[[nodiscard]] const char* GetName() const { return name; }

	[[nodiscard]] BE::PersistentAllocatorReference GetPersistentAllocator() const
	{
		return BE::PersistentAllocatorReference(GetName());
	}

	[[nodiscard]] BE::TransientAllocatorReference GetTransientAllocator() const
	{
		return BE::TransientAllocatorReference(GetName());
	}

protected:
	[[nodiscard]] BE::Logger* getLogger() const;
	uint8 getThread() const;
	
private:
	const utf8* name = "Object";

};

#ifdef BE_DEBUG
#define BE_LOG_SUCCESS(...)		this->getLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::SUCCESS, __VA_ARGS__);
#define BE_LOG_MESSAGE(...)		this->getLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::MESSAGE, __VA_ARGS__);
#define BE_LOG_WARNING(...)		this->getLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::WARNING, __VA_ARGS__);
#define BE_LOG_ERROR(...)		this->getLogger()->PrintObjectLog(this, BE::Logger::VerbosityLevel::FATAL, __VA_ARGS__);
#define BE_LOG_LEVEL(Level)		this->getLogger()->SetMinLogLevel(Level);

//#define BE_BASIC_LOG_SUCCESS(...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::SUCCESS, __VA_ARGS__);
//#define BE_BASIC_LOG_MESSAGE(...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::MESSAGE, __VA_ARGS__);
//#define BE_BASIC_LOG_WARNING(...)	BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::WARNING, __VA_ARGS__);
//#define BE_BASIC_LOG_ERROR(...)		BE::Application::Get()->GetLogger()->PrintBasicLog(BE::Logger::VerbosityLevel::FATAL, __VA_ARGS__);
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