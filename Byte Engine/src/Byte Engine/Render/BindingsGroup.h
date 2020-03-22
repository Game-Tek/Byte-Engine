#pragma once

#include "Containers/Array.hpp"

#include <unordered_map>

#include "BindingsSetDescriptor.h"
#include "Containers/Id.h"
#include "RAPI/Bindings.h"
#include "RAPI/CommandBuffer.h"
#include "Containers/Pair.h"

#include "RenderingCore.h"

namespace RAPI {
    class CommandBuffer;
    class RenderDevice;
	class UniformBuffer;
}

class RenderGroupBase
{
	/**
     * \brief Defines how many instances can be rendered every instanced draw call.
     * Also used to switch bound buffers when rendering.
     * Usually determined by the per instance data size and the max buffer size.\n
     * max buffer size/per instance data size = maxInstanceCount.
     */
    uint32 maxInstanceCount = 0;

    Array<Id64, 8>parentGroups;

protected:
    static Pair<RAPI::BindingsPoolCreateInfo, RAPI::BindingsSetCreateInfo> bindingDescriptorToRAPIBindings(const BindingsSetDescriptor& bindingsSetDescriptor);
	
public:

    void SetMaxInstanceCount(const uint32 instanceCount) { maxInstanceCount = instanceCount; }
	[[nodiscard]] uint32 GetMaxInstanceCount() const { return maxInstanceCount; }
	
    void AddParentGroup(const Id64& parentId) { parentGroups.push_back(parentId); }
	[[nodiscard]] const decltype(parentGroups)& GetParentGroups() const { return parentGroups; }
};

class BindingsGroup : public RenderGroupBase
{
    RAPI::BindingsPool* bindingsPool = nullptr;
    RAPI::BindingsSet* bindingsSet = nullptr;
	
public:
    struct BindingsGroupCreateInfo
    {
        RAPI::RenderDevice* RenderDevice = nullptr;
        BindingsSetDescriptor& BindingsSetDescriptor;
        uint8 MaxFramesInFlight = 0;
    };
    explicit BindingsGroup(const BindingsGroupCreateInfo& bindingsGroupCreateInfo);

    struct BindingsGroupBindInfo
    {
        RAPI::CommandBuffer* CommandBuffer = nullptr;
    };
    void Bind(const BindingsGroupBindInfo& bindInfo) const;
};

class BindingsGroupManager
{
    std::unordered_map<Id64::HashType, BindingsGroup> bindingsGroups;
	
    uint8 maxFramesInFlight = 0;
	
public:
    struct BindingsGroupManagerCreateInfo
    {
        uint8 MaxFramesInFlight = 0;
    };
    explicit BindingsGroupManager(const BindingsGroupManagerCreateInfo& createInfo) : maxFramesInFlight(createInfo.MaxFramesInFlight)
    {
    }
	
    const BindingsGroup& AddBindingsGroup(const Id64& bindingsGroupId, const BindingsGroup::BindingsGroupCreateInfo& bindingsGroupCreateInfo);
    [[nodiscard]] const BindingsGroup& GetBindingsGroup(const Id64& bindingsGroupId) const { return bindingsGroups.at(bindingsGroupId); }
	
    struct BindBindingsGroupInfo
    {
        RAPI::CommandBuffer* CommandBuffer = nullptr;
        Id64 BindingsGroup = 0;
    };
    void BindBindingsGroup(const BindBindingsGroupInfo& bindBindingsGroupInfo);

    struct BindDependencyGroupInfo
    {
        Id64 DependencyGroup = 0;
    };
    void BindDependencyGroups(const BindDependencyGroupInfo& bindDependencyGroupInfo);
	
};