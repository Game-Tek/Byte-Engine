#pragma once

#include "Core.h"

#include "RAPI/RenderMesh.h"

#include "RAPI/Vulkan/Vulkan.h"

class VulkanRenderMesh final : public RAPI::RenderMesh
{
	VkBuffer buffer = nullptr;
	VkDeviceMemory memory = nullptr;
	size_t indexBufferOffset = 0;

public:
	VulkanRenderMesh(class VulkanRenderDevice* vulkanRenderDevice, const RAPI::RenderMesh::RenderMeshCreateInfo& renderMeshCreateInfo);
	~VulkanRenderMesh() = default;

	void Destroy(class RAPI::RenderDevice* renderDevice) override;

	VkBuffer GetVkBuffer() const { return buffer; }
	size_t GetIndexBufferOffset() const { return indexBufferOffset; }
};
