#pragma once

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Logger.h"

//Assert
#ifdef BE_DEBUG
#define BE_ASSERT(func, text) if (!(func)) [[unlikely]] { __debugbreak(); }
#else
#define BE_ASSERT(func, text)
#endif