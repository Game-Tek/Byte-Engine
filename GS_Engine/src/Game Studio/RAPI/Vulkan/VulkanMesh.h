#pragma once

#include "Core.h"

#include "RAPI/Mesh.h"

#include "Native/VKBuffer.h"
#include "Native/VKMemory.h"

class VKDevice;

GS_CLASS VulkanMesh final : public Mesh
{
	VKBuffer VertexBuffer;
	VKMemory VBMemory;
	VKBuffer IndexBuffer;
	VKMemory IBMemory;
public:
	VulkanMesh(const VKDevice& _Device, const VKCommandPool& _CP, void* _VertexData, size_t _VertexDataSize, uint16* _IndexData, uint16 _IndexCount);
	~VulkanMesh() = default;

	INLINE const VKBuffer& GetVertexBuffer() const { return VertexBuffer; }
	INLINE const VKBuffer& GetIndexBuffer() const { return IndexBuffer; }
};
