#include "BindingsGroup.h"

BindingsGroup::BindingsGroup(const BindingsGroupCreateInfo& bindingsGroupCreateInfo)
{
}

void BindingsGroup::Bind(const BindingsGroupBindInfo& bindInfo) const
{
	
}

BindingsDependencyGroup::BindingsDependencyGroup(const BindingsDependencyGroupCreateInfo& BindingsDependencyGroupCreateInfo)
{
}

void BindingsDependencyGroup::Bind(const BindingsDependencyGroupBindInfo& bindInfo) const
{
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

const BindingsDependencyGroup& BindingsGroupManager::AddBindingsDependencyGroup(const Id& bindingsGroupId, const BindingsDependencyGroup::BindingsDependencyGroupCreateInfo& bindingsDependecyGroupCreateInfo)
{
	auto bg = bindingsDependencyGroups.emplace(bindingsGroupId, bindingsDependecyGroupCreateInfo);
	GS_ASSERT(!bg.second, "The Binding Dependency Group could not be inserted! Either the binding dependency group already exists or a hash collision ocurred.")

	uint32 max = 0;
	for (auto& e : bg.first->second.GetParentGroups())
	{
		auto ic = bindingsDependencyGroups.at(e).GetMaxInstanceCount();
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

void BindingsGroupManager::BindDependencyGroups(const BindDependencyGroupInfo& bindDependencyGroupInfo)
{
	auto bg = bindingsDependencyGroups.at(bindDependencyGroupInfo.DependencyGroup);

	BindingsDependencyGroup::BindingsDependencyGroupBindInfo bind_info;

	bg.Bind(bind_info);

	for (auto& g : bg.GetParentGroups())
	{
		bindingsDependencyGroups.at(g).Bind(bind_info);
	}
}
