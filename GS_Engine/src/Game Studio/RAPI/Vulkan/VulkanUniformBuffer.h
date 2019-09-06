#pragma once

#include "Core.h"

#include "RAPI/UniformBuffer.h"

#include "Native/VKBuffer.h"
#include "Native/VKMemory.h"

GS_CLASS VulkanUniformBuffer : public UniformBuffer
{
	VKBuffer Buffer;
	VKMemory Memory;

	void* MappedMemoryPointer = nullptr;

	static VKBufferCreator CreateBuffer(VKDevice* _Device, const UniformBufferCreateInfo& _BCI);
	VKMemoryCreator CreateMemory(VKDevice* _Device);
public:
	VulkanUniformBuffer(VKDevice* _Device, const UniformBufferCreateInfo& _BCI);
	~VulkanUniformBuffer();

	void UpdateBuffer(const UniformBufferUpdateInfo& _BUI) const override;

	[[nodiscard]] const VKBuffer& GetVKBuffer() const { return Buffer; }
};