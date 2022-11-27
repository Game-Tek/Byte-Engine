#pragma once

#include "ByteEngine/Debug/Logger.h"

#if BE_PLATFORM_WINDOWS
#define BE_DEBUG_BREAK __debugbreak()
#elif BE_PLATFORM_LINUX
#define BE_DEBUG_BREAK __builtin_trap()
#endif

//Assert
#if BE_DEBUG
#define BE_ASSERT(func, text) if (!(func)) [[unlikely]] { BE_DEBUG_BREAK; }
#else
#define BE_ASSERT(func, text)
#endif