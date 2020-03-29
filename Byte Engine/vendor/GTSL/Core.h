#pragma once

using byte = unsigned char;
using uint8 = unsigned char;
using int8 = char;
using uint16 = unsigned short;
using int16 = short;
using uint32 = unsigned int;
using int32 = int;
using uint64 = unsigned long long;
using int64 = long long;

#ifdef _DEBUG
#define GTSL_ASSERT(condition, text)
#else
#define GTSL_ASSERT(consdition, text)
#endif
