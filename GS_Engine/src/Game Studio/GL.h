#pragma once

#include "Logger.h"

#include "glad.h"

#ifdef GS_DEBUG
	#define GS_GL_CALL(func)	func;\
								Logger::GetglGetError();
#else
	#define GS_GL_CALL(func)	func;
#endif