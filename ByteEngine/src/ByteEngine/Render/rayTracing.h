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

	CommandBuffer::TraceRaysInfo traceRaysInfo;
	traceRaysInfo.DispatchSize = { 1280, 720, 0 };
	traceRaysInfo.RayGenDescriptor.Size = 32;
	traceRaysInfo.RayGenDescriptor.Offset = 0;
	traceRaysInfo.RayGenDescriptor.Buffer;
	traceRaysInfo.RayGenDescriptor.Stride = 32;

	traceRaysInfo.HitDescriptor.Size = 32;
	traceRaysInfo.HitDescriptor.Offset = 0;
	traceRaysInfo.HitDescriptor.Buffer;
	traceRaysInfo.HitDescriptor.Stride = 32;

	traceRaysInfo.MissDescriptor.Size = 32;
	traceRaysInfo.MissDescriptor.Offset = 0;
	traceRaysInfo.MissDescriptor.Buffer;
	traceRaysInfo.MissDescriptor.Stride = 32;
	
	commandBuffer.TraceRays(traceRaysInfo);
}