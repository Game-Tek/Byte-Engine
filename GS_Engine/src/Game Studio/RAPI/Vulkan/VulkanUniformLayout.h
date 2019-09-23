#pragma once

#include "Core.h"

#include "RAPI/UniformLayout.h"

#include "Native/VKDescriptorSetLayout.h"
#include "Native/VKDescriptorPool.h"
#include "Containers/DArray.hpp"
#include "Native/VKDespcriptorSet.h"
#include "Native/VKPipelineLayout.h"

struct VkWriteDescriptorSet;

class GS_API VulkanUniformLayout final : public UniformLayout
{
	VKDescriptorSetLayout DescriptorSetLayout;
	VKDescriptorPool DescriptorPool;
	DArray<VkDescriptorSet> DescriptorSets;
	VKPipelineLayout PipelineLayout;

	static VKDescriptorSetLayoutCreator CreateDescriptorSetLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI);

	static VKDescriptorPoolCreator CreateDescriptorPool(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI);
	VKPipelineLayoutCreator CreatePipelineLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI) const;

	void CreateDescriptorSet(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI);

public:
	VulkanUniformLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI);
	~VulkanUniformLayout() = default;

	void UpdateDescriptorSet(VKDevice* _Device, const UniformLayoutUpdateInfo& _ULUI);

	[[nodiscard]] const auto& GetVkDescriptorSets() const { return DescriptorSets; }
	[[nodiscard]] const VKDescriptorSetLayout& GetVKDescriptorSetLayout() const { return DescriptorSetLayout; }
	[[nodiscard]] const VKPipelineLayout& GetVKPipelineLayout() const { return PipelineLayout; }
};
