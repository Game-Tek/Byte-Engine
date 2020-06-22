#pragma once

// TYPEDEFS

using byte = unsigned char;
using uint8 = unsigned char;
using int8 = char;
using uint16 = unsigned short;
using int16 = short;
using uint32 = unsigned int;
using int32 = int;
using uint64 = unsigned long long;
using int64 = long long;

using UTF8 = char;

using float32 = float;
using float64 = double;

#ifdef BE_DEBUG
#define BE_DEBUG_ONLY(...) __VA_ARGS__;
#else
#define BE_DEBUG_ONLY(...)
#endif