#pragma once

#include "Logger.h"

#include "glad.h"

#ifdef GS_DEBUG
	#define GS_GL_CALL(func)	func;\
								GS_LOG_WARNING(Logger::GetglGetError(), ", ", #func, ", ", __LINE__);
#else
	#define GS_GL_CALL(func)	func;
#endif