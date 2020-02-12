#pragma once


// TYPEDEFS

typedef unsigned char byte;
typedef unsigned char uint8;
typedef char int8;
typedef unsigned short uint16;
typedef short int16;
typedef unsigned int uint32;
typedef int int32;
typedef unsigned long uint64;
typedef long int64;
typedef unsigned long long uint_64;
typedef long long int_64;

using GS_HASH_TYPE = uint_64;

#ifdef GS_PLATFORM_WIN
#define INLINE __forceinline
#else
	#define inline
#endif

#include "Debug/Logger.h"

//Assert
#ifdef GS_DEBUG
#define GS_ASSERT(func, text) if ((func)) { GS_BASIC_LOG_ERROR("ASSERT: File: %s, Line: %s: ", __FILE__, __LINE__,text); __debugbreak(); }
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

//  CASTS

#define DCAST(to, from) dynamic_cast<to>(from)
#define SCAST(to, from) static_cast<to>(from)
#define RCAST(to, from) reinterpret_cast<to>(from)
#define CCAST(to, from) const_cast<to>(from)

#define GS_ALIGN(x) __declspec(align(x))

constexpr uint8 uint8MAX = 0xff;
constexpr uint_64 uint_64MAX = 0xffffffffffffffff;
