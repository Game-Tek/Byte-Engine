#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

#include "Containers/Tuple.h"
#include "Containers/FVector.hpp"
#include "Containers/DArray.hpp"

MAKE_VK_HANDLE(VkRenderPass)

struct VkAttachmentDescription;
struct VkSubpassDescription;
struct VkSubpassDependency;

struct VkRenderPassCreateInfo;

GS_STRUCT Vk_RenderPassCreateInfo : VulkanObjectCreateInfo
{
	VkRenderPass RenderPass = VK_NULL_HANDLE;
};

GS_CLASS Vk_RenderPass final : public VulkanObject
{
	VkRenderPass RenderPass = nullptr;


public:
	static Vk_RenderPassCreateInfo CreateVk_RenderPassCreateInfo(const Vk_Device& _Device, const VkRenderPassCreateInfo* _VkRPCI);

	explicit Vk_RenderPass(const Vk_RenderPassCreateInfo& _Vk_RenderPassCreateInfo);

	~Vk_RenderPass();

	INLINE operator VkRenderPass() const { return RenderPass; }
};