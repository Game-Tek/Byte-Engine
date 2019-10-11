#pragma once

// TYPEDEFS

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
#ifndef GS_PRECISION_DOUBLE
typedef float real;
#else
typedef double real;
#endif

#ifdef GS_PLATFORM_WIN
	#define INLINE __forceinline
#else
	#define inline
#endif

//Library import/export.

#ifdef GS_PLATFORM_WIN
	#ifdef GS_BUILD
		#define GS_API //__declspec(dllexport)
	#else
		#define GS_API //__declspec(dllimport)
	#endif
#endif

#ifdef GS_PLATFORM_WIN
	#ifdef GS_BUILD
		#define GS_EXPORT_ONLY __declspec(dllexport)
	#else
		#define GS_EXPORT_ONLY
	#endif
#endif

//Class setup simplification.

#define GS_CLASS class GS_API

#define GS_STRUCT struct GS_API

//Assert
#ifdef GS_DEBUG
	#define GS_ASSERT(func, ...) func;\
							if ((func)) __debugbreak()
#else
	#define GS_ASSERT(func, ...) func;
#endif

#ifdef GS_DEBUG
#define GS_DEBUG_ONLY(...) __VA_ARGS__;
#else
#define GS_DEBUG_ONLY(...);
#endif

//  CASTS

#define DCAST(to, from) dynamic_cast<to>(from)
#define SCAST(to, from) static_cast<to>(from)
#define RCAST(to, from) reinterpret_cast<to>(from)
#define CCAST(to, from) const_cast<to>(from)

constexpr uint8 uint8MAX = 0xff;
constexpr uint_64 uint_64MAX = 0xffffffffffffffff;