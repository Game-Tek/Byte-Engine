#pragma once

#include "Logger.h"

//Library import/export.

#ifdef GS_PLATFORM_WIN
	#ifdef GS_BUILD
		#define GS_API __declspec(dllexport)
	#else
		#define GS_API __declspec(dllimport)
	#endif
#endif


//Class setup simplification.

#define GS_CLASS class GS_API


//Assert
#ifdef GS_DEBUG
	#define GS_ASSERT(func) func;\
							(!(func)) ? GS_LOG_ERROR("Function: ", #func, "\n", "File ", __FILE__, "\n", "Line: ", __LINE__)
	#else
	#define GS_ASSERT(func) func;
#endif

