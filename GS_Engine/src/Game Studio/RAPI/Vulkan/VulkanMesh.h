#pragma once

#include "Core.h"

#include "RAPI/Mesh.h"
#include "VulkanBuffers.h"

class Vk_Device;

GS_CLASS VulkanMesh final : public Mesh
{
	Vk_Buffer VertexBuffer;
	Vk_Memory VBMemory;
	Vk_Buffer IndexBuffer;
	Vk_Memory IBMemory;
public:
	VulkanMesh(const Vk_Device& _Device);
	~VulkanMesh() = default;

	INLINE const Vk_Buffer& GetVertexBuffer() const { return VertexBuffer; }
	INLINE const Vk_Buffer& GetIndexBuffer() const { return IndexBuffer; }
};
