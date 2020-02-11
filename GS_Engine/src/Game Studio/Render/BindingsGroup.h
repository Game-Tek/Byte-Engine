#pragma once

#include "Containers/Array.hpp"

#include <unordered_map>

#include "BindingsSetDescriptor.h"
#include "Containers/Id.h"
#include "RAPI/Bindings.h"
#include "RAPI/CommandBuffer.h"

class RenderGroupBase
{
	/**
     * \brief Defines how many instances can be rendered every instanced draw call.
     * Also used to switch bound buffers when rendering.
     * Usually determined by the per instance data size and the max buffer size.\n
     * max buffer size/per instance data size = maxInstanceCount.
     */
    uint32 maxInstanceCount = 0;

    Array<Id, 8>parentGroups;

public:

    void AddParentGroup(const Id& parentId) { parentGroups.push_back(parentId); }
	[[nodiscard]] const decltype(parentGroups)& GetParentGroups() const { return parentGroups; }
};

class BindingsGroup : public RenderGroupBase
{
    RAPI::BindingsPool* bindingsPool = nullptr;
    RAPI::BindingsSet* bindingsSet = nullptr;

public:
    struct BindingsGroupCreateInfo
    {
        BindingsSetDescriptor BindingsSetDescriptor;
    };
	
    explicit BindingsGroup(const BindingsGroupCreateInfo& bindingsGroupCreateInfo);
};

class BindingsGroupManager
{
    std::unordered_map<Id, BindingsGroup> bindingsGroups;
	
public:
	const BindingsGroup& AddBindingsGroup(const Id& bindingsGroupId, const BindingsGroup::BindingsGroupCreateInfo& bindingsGroupCreateInfo) { return bindingsGroups.emplace(bindingsGroupId, bindingsGroupCreateInfo).first->second; }
    const BindingsGroup& GetBindingsGroup(const Id& bindingsGroupId)
	{
		const auto find_result = bindingsGroups.find(bindingsGroupId);
        //GS_ASSERT(find_result);
        return find_result->second;
	}
	
    struct BindBindingsGroupInfo
    {
        RAPI::CommandBuffer* CommandBuffer = nullptr;
        Id BindingsGroup = 0;
    };
    void BindBindingsGroup(const BindBindingsGroupInfo& bindBindingsGroupInfo);
	
};