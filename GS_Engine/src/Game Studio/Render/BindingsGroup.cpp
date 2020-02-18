#include "BindingsGroup.h"

#include "RAPI/RenderDevice.h"

#include "RAPI/CommandBuffer.h"

Pair<RAPI::BindingsPoolCreateInfo, RAPI::BindingsSetCreateInfo> RenderGroupBase::bindingDescriptorToRAPIBindings(const BindingsSetDescriptor& bindingsSetDescriptor)
{
	RAPI::BindingsPoolCreateInfo bindings_pool_create_info;
	RAPI::BindingsSetCreateInfo bindings_set_create_info;
	
	for (auto& e : bindingsSetDescriptor)
	{
		RAPI::BindingDescriptor binding_descriptor;
		binding_descriptor.ArrayLength = e.Count;
		binding_descriptor.BindingType = e.Type;
		binding_descriptor.ShaderStage = bindingsSetDescriptor.GetShaderType();
		
		bindings_pool_create_info.BindingsSetLayout.emplace_back(binding_descriptor);

		bindings_set_create_info.BindingsSetLayout.emplace_back(binding_descriptor);
	}
	
	return { bindings_pool_create_info, bindings_set_create_info };
}

BindingsGroup::BindingsGroup(const BindingsGroupCreateInfo& bindingsGroupCreateInfo)
{
	auto pair = bindingDescriptorToRAPIBindings(bindingsGroupCreateInfo.BindingsSetDescriptor);

	pair.First.RenderDevice = bindingsGroupCreateInfo.RenderDevice;
	pair.First.BindingsSetCount = bindingsGroupCreateInfo.MaxFramesInFlight;

	bindingsPool = bindingsGroupCreateInfo.RenderDevice->CreateBindingsPool(pair.First);

	pair.Second.RenderDevice = bindingsGroupCreateInfo.RenderDevice;
	pair.Second.BindingsSetCount = bindingsGroupCreateInfo.MaxFramesInFlight;
	pair.Second.BindingsPool = bindingsPool;

	bindingsSet = bindingsGroupCreateInfo.RenderDevice->CreateBindingsSet(pair.Second);
}

void BindingsGroup::Bind(const BindingsGroupBindInfo& bindInfo) const
{
	RAPI::CommandBuffer::BindBindingsSetInfo bind_bindings_set_info;

	FVector<RAPI::BindingsSet*> a(1, &bindingsSet);
	bind_bindings_set_info.BindingsSets = &a;
	
	bindInfo.CommandBuffer->BindBindingsSet(bind_bindings_set_info);
}

const BindingsGroup& BindingsGroupManager::AddBindingsGroup(const Id& bindingsGroupId, const BindingsGroup::BindingsGroupCreateInfo& bindingsGroupCreateInfo)
{
	auto bg = bindingsGroups.emplace(bindingsGroupId, bindingsGroupCreateInfo);
	GS_ASSERT(!bg.second, "The Binding Group could not be inserted! Either the binding group already exists or a hash collision ocurred.")

	uint32 max = 0;
	for (auto& e : bg.first->second.GetParentGroups())
	{
		auto ic = bindingsGroups.at(e).GetMaxInstanceCount();
		ic > max ? max = ic : 0;
	}

	bg.first->second.SetMaxInstanceCount(max);

	return bg.first->second;
}

void BindingsGroupManager::BindBindingsGroup(const BindBindingsGroupInfo& bindBindingsGroupInfo)
{
	auto bg = bindingsGroups.at(bindBindingsGroupInfo.BindingsGroup);

	BindingsGroup::BindingsGroupBindInfo bind_info;

	bg.Bind(bind_info);
	
	for (auto& g : bg.GetParentGroups())
	{
		bindingsGroups.at(g).Bind(bind_info);
	}
}
