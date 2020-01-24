#pragma once

#include "Core.h"

#include "RAPI/UniformLayout.h"

#include "Native/VKDescriptorSetLayout.h"
#include "Native/VKDescriptorPool.h"
#include "Native/VKDespcriptorSet.h"
#include "Native/VKPipelineLayout.h"
#include "Containers/FVector.hpp"

struct VkWriteDescriptorSet;

class GS_API VulkanUniformLayout final : public UniformLayout
{
	VkDescriptorSetLayout descriptorSetLayout;
	VkDescriptorPool descriptorPool;
	FVector<VkDescriptorSet> descriptorSets;
	VkPipelineLayout pipelineLayout;

	static VKDescriptorSetLayoutCreator CreateDescriptorSetLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI);

	static VKDescriptorPoolCreator CreateDescriptorPool(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI);
	VKPipelineLayoutCreator CreatePipelineLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI) const;

	void CreateDescriptorSet(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI);

public:
	VulkanUniformLayout(VKDevice* _Device, const UniformLayoutCreateInfo& _PLCI);
	~VulkanUniformLayout() = default;

	void UpdateUniformSet(const UniformLayoutUpdateInfo& _ULUI) override;

	[[nodiscard]] const auto& GetVkDescriptorSets() const { return descriptorSets; }
	[[nodiscard]] const VkDescriptorSetLayout& GetVkDescriptorSetLayout() const { return descriptorSetLayout; }
	[[nodiscard]] const VkPipelineLayout& GetVkPipelineLayout() const { return pipelineLayout; }
};
