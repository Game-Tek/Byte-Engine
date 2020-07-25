#include "ByteEngine/Object.h"

#include "ByteEngine/Application/Application.h"

BE::Logger* Object::getLogger() const { return BE::Application::Get()->GetLogger(); }
