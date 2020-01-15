#include "FileSystem.h"

#if GS_PLATFORM_WIN
#include <windows.h>
#endif

#define INCLDIF(x, y)
#if x
#include y
#else
#define INCLDIF(x, y)
#endif

FString FileSystem::GetRunningPath()
{
	char a[512];
	GetModuleFileNameA(NULL, a, 512);
	FString result(a);
	result.Drop(result.FindLast('/'));
	return result;
}
