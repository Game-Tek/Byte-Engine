#pragma once

#include "Core.h"

#include "RAPI/Mesh.h"

#include "Native/Vk_Buffer.h"
#include "Native/Vk_Memory.h"

class Vk_Device;

GS_CLASS VulkanMesh final : public Mesh
{
	Vk_Buffer VertexBuffer;
	Vk_Memory VBMemory;
	Vk_Buffer IndexBuffer;
	Vk_Memory IBMemory;
public:
	VulkanMesh(const Vk_Device& _Device, const Vk_CommandPool& _CP, void* _VertexData, size_t _VertexDataSize, uint16* _IndexData, uint16 _IndexCount);
	~VulkanMesh() = default;

	INLINE const Vk_Buffer& GetVertexBuffer() const { return VertexBuffer; }
	INLINE const Vk_Buffer& GetIndexBuffer() const { return IndexBuffer; }
};
