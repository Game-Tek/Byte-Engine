#include "Object.h"

#include <GTSL/Thread.hpp>

#include "Application/Application.h"

BE::Logger* Object::GetLogger() const
{
	return BE::Application::Get()->GetLogger();
}

GTSL::uint8 Object::GetThread() const
{
	return GTSL::Thread::ThisTreadID();
}
