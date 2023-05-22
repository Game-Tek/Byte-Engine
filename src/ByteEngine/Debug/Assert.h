#pragma once

#include "Logger.h"

#if BE_PLATFORM_WINDOWS
#define BE_DEBUG_BREAK __debugbreak()
#elif BE_PLATFORM_LINUX
#include <signal.h>
#define BE_DEBUG_BREAK raise(SIGTRAP)
#endif

#if BE_DEBUG
#define BE_ASSERT(func, text) if (!(func)) [[unlikely]] { BE_DEBUG_BREAK; }
#else
#define BE_ASSERT(func, text)
#endif