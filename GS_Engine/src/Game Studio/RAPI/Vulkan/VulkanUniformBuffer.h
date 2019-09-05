#pragma once

#include "Core.h"

#include "RAPI/Buffer.h"

#include "Native/VKBuffer.h"

GS_CLASS VulkanUniformBuffer : public Buffer
{
	VKBuffer Buffer;
public:
	VulkanUniformBuffer(VKDevice* _Device, const BufferCreateInfo& _BCI);
	~VulkanUniformBuffer() = default;

	[[nodiscard]] const VKBuffer& GetVKBuffer() const { return Buffer; }
};