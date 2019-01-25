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

#ifdef GS_PLATFORM_WIN
	#define INLINE __forceinline
#endif

//Class setup simplification.

#define GS_CLASS class GS_API

#define GS_STRUCT struct GS_API

//Library import/export.

#ifdef GS_PLATFORM_WIN
	#ifdef GS_BUILD
		#define GS_API __declspec(dllexport)
	#else
		#define GS_API __declspec(dllimport)
	#endif
#endif

//Assert
#ifdef GS_DEBUG
	#define GS_ASSERT(func) func;\
							if (!(func)) __debugbreak()
#else
	#define GS_ASSERT(func) func;
#endif