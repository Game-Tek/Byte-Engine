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

using GS_HASH_TYPE = uint64;

#ifdef GS_PLATFORM_WIN
#define INLINE __forceinline
#else
	#define inline
#endif

#include "Debug/Logger.h"

//Assert
#ifdef GS_DEBUG
#define GS_ASSERT(func, text) if ((func)) { GS_BASIC_LOG_ERROR("ASSERT: File: %s, Line: %s: %s", __FILE__, __LINE__, text); __debugbreak(); }
#else
	#define GS_ASSERT(func, text)
#endif

#ifdef GS_DEBUG
#define GS_DEBUG_ONLY(...) __VA_ARGS__;
#else
#define GS_DEBUG_ONLY(...)
#endif

#ifdef GS_DEBUG
#define GS_THROW(text) throw (text);
#else
#define GS_THROW(text)
#endif

constexpr uint8 uint8MAX = 0xff;
constexpr uint64 uint_64MAX = 0xffffffffffffffff;
