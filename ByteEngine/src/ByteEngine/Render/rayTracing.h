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

inline void build()
{
	AccelerationStructure accelerationStructure;

	GAL::BuildAccelerationStructureInfo buildAccelerationStructureInfo;
	buildAccelerationStructureInfo.Flags = AccelerationStructureFlags::PREFER_FAST_TRACE;
	buildAccelerationStructureInfo.Update = false;
	buildAccelerationStructureInfo.Count = 1;/*number of acc. structures to build*/
	buildAccelerationStructureInfo.SourceAccelerationStructure = accelerationStructure;
	buildAccelerationStructureInfo.DestinationAccelerationStructure = accelerationStructure;
	buildAccelerationStructureInfo.IsTopLevelFalse = false;
	buildAccelerationStructureInfo.ScratchBufferAddress = 0;
	buildAccelerationStructureInfo.Geometries = nullptr;

	//AccelerationStructure::TopLevelCreateInfo topLevelCreateInfo;
	//topLevelCreateInfo.RenderDevice;
	//if constexpr (_DEBUG) { topLevelCreateInfo.Name = "Top Level Acc. Structure"; }
	//topLevelCreateInfo.Flags = GAL::VulkanAccelerationStructureFlags::PREFER_FAST_TRACE;
	//topLevelCreateInfo.CompactedSize = 0;
	//topLevelCreateInfo.MaxGeometryCount = 0;
	//topLevelAccelerationStructure.Initialize(topLevelCreateInfo);
	
	{
		CommandBuffer::AddPipelineBarrierInfo addPipelineBarrierInfo;
		addPipelineBarrierInfo.InitialStage = PipelineStage::ACCELERATION_STRUCTURE_BUILD;
		addPipelineBarrierInfo.FinalStage = PipelineStage::ACCELERATION_STRUCTURE_BUILD;
	
		GTSL::Array<CommandBuffer::MemoryBarrier, 1> memoryBarriers(1);
		memoryBarriers[0].SourceAccessFlags = AccessFlags::ACCELERATION_STRUCTURE_WRITE;
		memoryBarriers[0].DestinationAccessFlags = AccessFlags::ACCELERATION_STRUCTURE_READ;
		
		addPipelineBarrierInfo.MemoryBarriers = memoryBarriers;		
	}
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