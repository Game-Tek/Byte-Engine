#pragma once

#include "RAPI/GraphicsPipeline.h"

#include <unordered_map>
#include "Containers/FVector.hpp"

class BindingsSetDescriptor;

class RenderGroupBase
{
	/**
     * \brief Defines how many instances can be rendered every instanced draw call.
     * Also used to switch bound buffers when rendering.
     * Usually determined by the per instance data size and the max buffer size.\n
     * max buffer size/per instance data size = maxInstanceCount.
     */
    uint32 maxInstanceCount = 0;

    Id parentGroup = 0;

public:

    void SetParentGroup(const Id& parentId) { parentGroup = parentId; }
	[[nodiscard]] Id GetParentGroup() const { return parentGroup; }
};

class BindingsGroup : public RenderGroupBase
{
    RAPI::BindingsPool* bindingsPool = nullptr;
    RAPI::BindingsSet* bindingsSet = nullptr;

public:
    struct BindingsGroupCreateInfo
    {
	    
    };
	
    explicit BindingsGroup(const BindingsGroupCreateInfo& bindingsGroupCreateInfo);
};

class BindingsGroupManager
{
    std::unordered_map<Id, BindingsGroup> bindingsGroups;
	
public:
	void AddBindingsGroup(const Id& bindingsGroupId, const BindingsGroup& bindingsGroup) { bindingsGroups.emplace(bindingsGroupId, bindingsGroup); }

    struct BindBindingsGroupInfo
    {
        RAPI::CommandBuffer* CommandBuffer = nullptr;
        Id BindingsGroup = 0;
    };
    void BindBindingsGroup(const BindBindingsGroupInfo& bindBindingsGroupInfo);
	
};