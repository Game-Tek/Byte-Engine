#pragma once

#include "RenderTypes.h"

#undef MemoryBarrier

inline void queries()
{
	//We then use multiple command buffers to launch all the BLAS builds. We are using multiple command buffers instead of one,
	//to allow the driver to allow system interuption and avoid a TDR if the job was to heavy.
	
	QueryPool::CreateInfo createInfo;
	createInfo.RenderDevice;
	createInfo.QueryType = QueryType::ACCELERATION_STRUCTURE_COMPACTED_SIZE;
	createInfo.QueryCount = 16;

	QueryPool queryPool(createInfo);
}

inline void trace()
{
	CommandBuffer commandBuffer;
}