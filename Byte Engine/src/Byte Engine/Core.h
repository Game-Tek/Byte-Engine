#pragma once

// TYPEDEFS

typedef unsigned char byte;
typedef unsigned char uint8;
typedef char int8;
typedef unsigned short uint16;
typedef short int16;
typedef unsigned int uint32;
typedef int int32;
typedef unsigned long long uint64;
typedef long long int64;

#ifdef BE_PLATFORM_WIN
#define INLINE __forceinline
#else
	#define inline
#endif

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

constexpr uint8 uint8MAX = 0xff;
constexpr uint64 uint_64MAX = 0xffffffffffffffff;
