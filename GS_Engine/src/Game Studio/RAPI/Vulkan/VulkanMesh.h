#pragma once

#include "Core.h"

#include "RAPI/RenderMesh.h"

#include "Native/VKBuffer.h"
#include "Native/VKMemory.h"

class VKDevice;

class GS_API VulkanMesh final : public RenderMesh
{
	VKBuffer VertexBuffer;
	VKMemory VBMemory;
	VKBuffer IndexBuffer;
	VKMemory IBMemory;

	static VKBufferCreator CreateVKBufferCreator(VKDevice* _Device, unsigned _BufferUsage, size_t _BufferSize);
	static VKMemoryCreator CreateVKMemoryCreator(VKDevice* _Device, VkMemoryRequirements _MemReqs, unsigned _MemoryProps);
public:
	VulkanMesh(VKDevice* _Device, const VKCommandPool& _CP, void* _VertexData, size_t _VertexDataSize, uint16* _IndexData, uint16 _IndexCount);
	~VulkanMesh() = default;

	INLINE const VKBuffer& GetVertexBuffer() const { return VertexBuffer; }
	INLINE const VKBuffer& GetIndexBuffer() const { return IndexBuffer; }
};
