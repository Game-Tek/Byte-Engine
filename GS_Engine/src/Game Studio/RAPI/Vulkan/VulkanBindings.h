#pragma once

#include "Core.h"

#include "Vulkan.h"

#include "RAPI/Bindings.h"

#include "Containers/Array.hpp"

struct VkWriteDescriptorSet;

class VulkanBindingsPool final : public BindingsPool
{
	VkDescriptorPool vkDescriptorPool = nullptr;

public:
	VulkanBindingsPool(class VulkanRenderDevice* device, const BindingsPoolCreateInfo& descriptorPoolCreateInfo);

	void FreeBindingsSet() override;
	void FreePool(const FreeBindingsPoolInfo& freeDescriptorPoolInfo) override;

	[[nodiscard]] VkDescriptorPool GetVkDescriptorPool() const { return vkDescriptorPool; }
};

class VulkanBindingsSet final : public BindingsSet
{
	VkDescriptorSetLayout vkDescriptorSetLayout = nullptr;
	Array<VkDescriptorSet, 4> vkDescriptorSets;

public:
	VulkanBindingsSet(class VulkanRenderDevice* device, const BindingsSetCreateInfo& descriptorSetCreateInfo);

	void Update(const BindingSetUpdateInfo& uniformLayoutUpdateInfo) override;

	[[nodiscard]] VkDescriptorSetLayout GetVkDescriptorSetLayout() const { return vkDescriptorSetLayout; }
	[[nodiscard]] const Array<VkDescriptorSet, 4>& GetVkDescriptorSets() const { return vkDescriptorSets; }
};
