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

    void SetMaxInstanceCount(const uint32 instanceCount) { maxInstanceCount = instanceCount; }
	[[nodiscard]] uint32 GetMaxInstanceCount() const { return maxInstanceCount; }
	
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

    struct BindingsGroupBindInfo
    {
	    
    };
    void Bind(const BindingsGroupBindInfo& bindInfo) const;
};

class BindingsDependencyGroup : public RenderGroupBase
{
    RAPI::BindingsPool* bindingsPool = nullptr;
    RAPI::BindingsSet* bindingsSet = nullptr;

public:
    struct BindingsDependencyGroupCreateInfo
    {
        BindingsSetDescriptor BindingsSetDescriptor;
    };

    explicit BindingsDependencyGroup(const BindingsDependencyGroupCreateInfo& BindingsDependencyGroupCreateInfo);

    struct BindingsDependencyGroupBindInfo
    {

    };
    void Bind(const BindingsDependencyGroupBindInfo& bindInfo) const;
};

class BindingsGroupManager
{
    std::unordered_map<Id::HashType, BindingsGroup> bindingsGroups;
    std::unordered_map<Id::HashType, BindingsDependencyGroup> bindingsDependencyGroups;
	
    uint8 maxFramesInFlight = 0;
	
public:
    struct BindingsGroupManagerCreateInfo
    {
        uint8 MaxFramesInFlight = 0;
    };
    explicit BindingsGroupManager(const BindingsGroupManagerCreateInfo& createInfo) : maxFramesInFlight(createInfo.MaxFramesInFlight)
    {
    }
	
    const BindingsGroup& AddBindingsGroup(const Id& bindingsGroupId, const BindingsGroup::BindingsGroupCreateInfo& bindingsGroupCreateInfo);
    [[nodiscard]] const BindingsGroup& GetBindingsGroup(const Id& bindingsGroupId) const { return bindingsGroups.at(bindingsGroupId); }

    const BindingsDependencyGroup& AddBindingsDependencyGroup(const Id& bindingsGroupId, const BindingsDependencyGroup::BindingsDependencyGroupCreateInfo& bindingsDependecyGroupCreateInfo);
    [[nodiscard]] const BindingsDependencyGroup& GetBindingsDependencyGroup(const Id& bindingsDependencyGroupId) const { return bindingsDependencyGroups.at(bindingsDependencyGroupId); }
	
    struct BindBindingsGroupInfo
    {
        RAPI::CommandBuffer* CommandBuffer = nullptr;
        Id BindingsGroup = 0;
    };
    void BindBindingsGroup(const BindBindingsGroupInfo& bindBindingsGroupInfo);

    struct BindDependencyGroupInfo
    {
        Id DependencyGroup = 0;
    };
    void BindDependencyGroups(const BindDependencyGroupInfo& bindDependencyGroupInfo);
	
};