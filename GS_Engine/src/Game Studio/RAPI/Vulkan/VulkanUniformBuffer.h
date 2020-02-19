#pragma once

#include "Core.h"

#include "RAPI/UniformBuffer.h"

#include "Native/VKBuffer.h"
#include "Native/VKMemory.h"

class VulkanUniformBuffer : public RAPI::UniformBuffer
{
	VKBuffer Buffer;
	VKMemory Memory;

	byte* MappedMemoryPointer = nullptr;

	static VKBufferCreator CreateBuffer(VKDevice* _Device, const RAPI::UniformBufferCreateInfo& _BCI);
	VKMemoryCreator CreateMemory(VKDevice* _Device);
public:
	VulkanUniformBuffer(VKDevice* _Device, const RAPI::UniformBufferCreateInfo& _BCI);
	~VulkanUniformBuffer();

	void UpdateBuffer(const RAPI::UniformBufferUpdateInfo& uniformBufferUpdateInfo) const override;

	[[nodiscard]] const VKBuffer& GetVKBuffer() const { return Buffer; }
};
