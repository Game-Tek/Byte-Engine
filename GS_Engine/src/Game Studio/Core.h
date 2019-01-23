#pragma once

// TYPEDEFS

typedef unsigned char UINT8;
typedef char INT8;
typedef unsigned short UINT16;
typedef short INT16;
typedef unsigned int UINT32;
typedef int INT32;
typedef unsigned long UINT64;
typedef long INT64;
typedef unsigned long long UINT_64;
typedef long long INT_64;

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