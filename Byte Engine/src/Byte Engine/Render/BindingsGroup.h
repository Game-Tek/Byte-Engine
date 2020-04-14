#pragma once

#include <GTSL/Array.hpp>

#include <unordered_map>

#include "BindingsSetDescriptor.h"
#include <GTSL/Id.h>
#include "GAL/Bindings.h"
#include "GAL/CommandBuffer.h"
#include <GTSL/Pair.h>

#include "RenderingCore.h"

namespace GAL {
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

    Array<GTSL::Id64, 8>parentGroups;

protected:
    static Pair<GAL::BindingsPoolCreateInfo, GAL::BindingsSetCreateInfo> bindingDescriptorToRAPIBindings(const BindingsSetDescriptor& bindingsSetDescriptor);
	
public:

    void SetMaxInstanceCount(const uint32 instanceCount) { maxInstanceCount = instanceCount; }
	[[nodiscard]] uint32 GetMaxInstanceCount() const { return maxInstanceCount; }
	
    void AddParentGroup(const GTSL::Id64& parentId) { parentGroups.push_back(parentId); }
	[[nodiscard]] const decltype(parentGroups)& GetParentGroups() const { return parentGroups; }
};

class BindingsGroup : public RenderGroupBase
{
    GAL::BindingsPool* bindingsPool = nullptr;
    GAL::BindingsSet* bindingsSet = nullptr;
	
public:
    struct BindingsGroupCreateInfo
    {
        GAL::RenderDevice* RenderDevice = nullptr;
        BindingsSetDescriptor& BindingsSetDescriptor;
        uint8 MaxFramesInFlight = 0;
    };
    explicit BindingsGroup(const BindingsGroupCreateInfo& bindingsGroupCreateInfo);

    struct BindingsGroupBindInfo
    {
        GAL::CommandBuffer* CommandBuffer = nullptr;
    };
    void Bind(const BindingsGroupBindInfo& bindInfo) const;
};

class BindingsGroupManager
{
    std::unordered_map<GTSL::Id64::HashType, BindingsGroup> bindingsGroups;
	
    uint8 maxFramesInFlight = 0;
	
public:
    struct BindingsGroupManagerCreateInfo
    {
        uint8 MaxFramesInFlight = 0;
    };
    explicit BindingsGroupManager(const BindingsGroupManagerCreateInfo& createInfo) : maxFramesInFlight(createInfo.MaxFramesInFlight)
    {
    }
	
    const BindingsGroup& AddBindingsGroup(const GTSL::Id64& bindingsGroupId, const BindingsGroup::BindingsGroupCreateInfo& bindingsGroupCreateInfo);
    [[nodiscard]] const BindingsGroup& GetBindingsGroup(const GTSL::Id64& bindingsGroupId) const { return bindingsGroups.at(bindingsGroupId); }
	
    struct BindBindingsGroupInfo
    {
        GAL::CommandBuffer* CommandBuffer = nullptr;
        GTSL::Id64 BindingsGroup = 0;
    };
    void BindBindingsGroup(const BindBindingsGroupInfo& bindBindingsGroupInfo);

    struct BindDependencyGroupInfo
    {
        GTSL::Id64 DependencyGroup = 0;
    };
    void BindDependencyGroups(const BindDependencyGroupInfo& bindDependencyGroupInfo);
	
};