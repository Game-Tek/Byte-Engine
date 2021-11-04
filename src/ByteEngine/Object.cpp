#include "ByteEngine/Object.h"

#include <GTSL/Thread.hpp>


#include "ByteEngine/Application/Application.h"

BE::Logger* Object::getLogger() const { return BE::Application::Get()->GetLogger(); }

uint8 Object::getThread() const { return GTSL::Thread::ThisTreadID(); }
