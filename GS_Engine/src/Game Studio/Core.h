#pragma once

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