#include "System.h"

#if GS_PLATFORM_WIN
#include <windows.h>
#endif

#define INCLDIF(x, y)
#if x
#include y
#else
#define INCLDIF(x, y)
#endif

FString System::GetRunningPath()
{
	char a[512];
	GetModuleFileNameA(NULL, a, 512);
	FString result(a);
	result.Drop(result.FindLast('\\') + 1);
	result.ReplaceAll('\\', '/');
	return result;
}

RAMInfo System::GetRAMInfo()
{
	LPMEMORYSTATUSEX memory_status{};
	GlobalMemoryStatusEx(memory_status);
	
	RAMInfo ram_info;
	ram_info.FreePhysicalMemory = memory_status->ullAvailPhys;
	ram_info.TotalPhysicalMemory = memory_status->ullTotalPhys;
	return ram_info;
}

VectorInfo System::GetVectorInfo()
{
	//https://stackoverflow.com/questions/6121792/how-to-check-if-a-cpu-supports-the-sse3-instruction-set
	
    bool OS_x64;
    bool OS_AVX;
    bool OS_AVX512;
	
    VectorInfo vector_info;
	
    int info[4];
    __cpuidex(info, 0, 0);
    int nIds = info[0];

    __cpuidex(info, 0x80000000, 0);
    uint32 nExIds = info[0];

    //  Detect Features
    if (nIds >= 0x00000001) 
    {
        __cpuidex(info, 0x00000001, 0);
        vector_info.HW_MMX = (info[3] & ((int)1 << 23)) != 0;
        vector_info.HW_SSE = (info[3] & ((int)1 << 25)) != 0;
        vector_info.HW_SSE2 = (info[3] & ((int)1 << 26)) != 0;
        vector_info.HW_SSE3 = (info[2] & ((int)1 << 0)) != 0;

        vector_info.HW_SSSE3 = (info[2] & ((int)1 << 9)) != 0;
        vector_info.HW_SSE41 = (info[2] & ((int)1 << 19)) != 0;
        vector_info.HW_SSE42 = (info[2] & ((int)1 << 20)) != 0;
        vector_info.HW_AES = (info[2] & ((int)1 << 25)) != 0;

        vector_info.HW_AVX = (info[2] & ((int)1 << 28)) != 0;
        vector_info.HW_FMA3 = (info[2] & ((int)1 << 12)) != 0;

        vector_info.HW_RDRAND = (info[2] & ((int)1 << 30)) != 0;
    }
    if (nIds >= 0x00000007)
    {
        __cpuidex(info, 0x00000007, 0);
        vector_info.HW_AVX2 = (info[1] & ((int)1 << 5)) != 0;

        vector_info.HW_BMI1 = (info[1] & ((int)1 << 3)) != 0;
        vector_info.HW_BMI2 = (info[1] & ((int)1 << 8)) != 0;
        vector_info.HW_ADX = (info[1] & ((int)1 << 19)) != 0;
        vector_info.HW_MPX = (info[1] & ((int)1 << 14)) != 0;
        vector_info.HW_SHA = (info[1] & ((int)1 << 29)) != 0;
        vector_info.HW_PREFETCHWT1 = (info[2] & ((int)1 << 0)) != 0;

        vector_info.HW_AVX512_F = (info[1] & ((int)1 << 16)) != 0;
        vector_info.HW_AVX512_CD = (info[1] & ((int)1 << 28)) != 0;
        vector_info.HW_AVX512_PF = (info[1] & ((int)1 << 26)) != 0;
        vector_info.HW_AVX512_ER = (info[1] & ((int)1 << 27)) != 0;
        vector_info.HW_AVX512_VL = (info[1] & ((int)1 << 31)) != 0;
        vector_info.HW_AVX512_BW = (info[1] & ((int)1 << 30)) != 0;
        vector_info.HW_AVX512_DQ = (info[1] & ((int)1 << 17)) != 0;
        vector_info.HW_AVX512_IFMA = (info[1] & ((int)1 << 21)) != 0;
        vector_info.HW_AVX512_VBMI = (info[2] & ((int)1 << 1)) != 0;
    }
    if (nExIds >= 0x80000001)
    {
        __cpuidex(info, 0x80000001, 0);
        vector_info.HW_x64 = (info[3] & ((int)1 << 29)) != 0;
        vector_info.HW_ABM = (info[2] & ((int)1 << 5)) != 0;
        vector_info.HW_SSE4a = (info[2] & ((int)1 << 6)) != 0;
        vector_info.HW_FMA4 = (info[2] & ((int)1 << 16)) != 0;
        vector_info.HW_XOP = (info[2] & ((int)1 << 11)) != 0;
    }

    return vector_info;
}
