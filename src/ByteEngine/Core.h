#pragma once

#include <GTSL/Core.h>

#define LSTR(x) u8 ## x

#if BE_DEBUG
#define BE_DEBUG_ONLY(...) __VA_ARGS__;
#else
#define BE_DEBUG_ONLY(...)
#endif