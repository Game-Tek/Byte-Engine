#pragma once

// TYPEDEFS

using byte = unsigned char;
using uint8 = unsigned char;
using int8 = char;
using uint16 = unsigned short;
using int16 = short;
using uint32 = unsigned int;
using int32 = int;
using uint64 = unsigned long long;
using int64 = long long;

#include "Debug/Logger.h"

//Assert
#ifdef BE_DEBUG
#define BE_ASSERT(func, text) if ((func)) { BE_BASIC_LOG_ERROR("ASSERT: File: %s, Line: %s: %s", __FILE__, __LINE__, text); __debugbreak(); }
#else
#define BE_ASSERT(func, text)
#endif

#ifdef BE_DEBUG
#define BE_DEBUG_ONLY(...) __VA_ARGS__;
#else
#define BE_DEBUG_ONLY(...)
#endif

#ifdef BE_DEBUG
#define BE_THROW(text) throw (text);
#else
#define BE_THROW(text)
#endif