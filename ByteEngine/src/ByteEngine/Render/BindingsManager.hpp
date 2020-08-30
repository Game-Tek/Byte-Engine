#pragma once

#include <GTSL/Ranger.h>

#include "RenderSystem.h"
#include "RenderTypes.h"

template<class ALLOCATOR>
class BindingsManager
{
public:
	BindingsManager(const ALLOCATOR& allocator, RenderSystem* rSys, CommandBuffer* cBuffer) : renderSystem(rSys), commandBuffer(cBuffer), boundBindingsPerSet(64, allocator)
	{
	}

	void AddBinding(BindingsSet binding, const PipelineType pipelineType, const PipelineLayout pipelineLayout)
	{
		CommandBuffer::BindBindingsSetInfo bindBindingsSetInfo;
		bindBindingsSetInfo.RenderDevice = renderSystem->GetRenderDevice();
		bindBindingsSetInfo.FirstSet = boundSets;
		bindBindingsSetInfo.BoundSets = 1;
		bindBindingsSetInfo.BindingsSets = GTSL::Ranger<BindingsSet>(1, &binding);
		bindBindingsSetInfo.PipelineLayout = &pipelineLayout;
		bindBindingsSetInfo.PipelineType = pipelineType;
		commandBuffer->BindBindingsSets(bindBindingsSetInfo);

		boundSets += 1;
		boundBindingsPerSet.EmplaceBack(1);
	}

	void AddBinding(BindingsSet binding, const GTSL::Ranger<const uint32> offsets, const PipelineType pipelineType, const PipelineLayout pipelineLayout)
	{
		CommandBuffer::BindBindingsSetInfo bindBindingsSetInfo;
		bindBindingsSetInfo.RenderDevice = renderSystem->GetRenderDevice();
		bindBindingsSetInfo.FirstSet = boundSets;
		bindBindingsSetInfo.BoundSets = 1;
		bindBindingsSetInfo.BindingsSets = GTSL::Ranger<BindingsSet>(1, &binding);
		bindBindingsSetInfo.PipelineLayout = &pipelineLayout;
		bindBindingsSetInfo.PipelineType = pipelineType;
		bindBindingsSetInfo.Offsets = offsets;
		commandBuffer->BindBindingsSets(bindBindingsSetInfo);

		boundSets += 1;
		boundBindingsPerSet.EmplaceBack(1);
	}

	void AddBindings(const GTSL::Ranger<BindingsSet> bindings, const PipelineType pipelineType, const PipelineLayout pipelineLayout)
	{
		CommandBuffer::BindBindingsSetInfo bindBindingsSetInfo;
		bindBindingsSetInfo.RenderDevice = renderSystem->GetRenderDevice();
		bindBindingsSetInfo.FirstSet = boundSets;
		bindBindingsSetInfo.BoundSets = bindings.ElementCount();
		bindBindingsSetInfo.BindingsSets = bindings;
		bindBindingsSetInfo.PipelineLayout = &pipelineLayout;
		bindBindingsSetInfo.PipelineType = pipelineType;
		commandBuffer->BindBindingsSets(bindBindingsSetInfo);

		boundSets += bindings.ElementCount();
		boundBindingsPerSet.EmplaceBack(bindings.ElementCount());
	}

	void PopBindings()
	{		
		boundSets -= boundBindingsPerSet.back();
		boundBindingsPerSet.PopBack();
	}
private:
	RenderSystem* renderSystem;
	CommandBuffer* commandBuffer;

	Vector<uint8, ALLOCATOR> boundBindingsPerSet;
	uint32 boundSets = 0;
};
