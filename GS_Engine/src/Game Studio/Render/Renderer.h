#pragma once

#include "Core.h"

class CommandBuffer;
class Semaphore;
class Swapchain;
class Surface;
class Fence;
class Mesh;

GS_CLASS Renderer
{
public:
	virtual ~Renderer();

	static Renderer* CreateRenderer();

	virtual CommandBuffer* CreateCommandBuffer() = 0;
	virtual Semaphore* CreteSemaphore() = 0;
	virtual Swapchain* CreateSwapchain() = 0;
	virtual Surface* CreateSurface() = 0;
	virtual Fence* CreteFence() = 0;
	virtual Mesh* CreateMesh() = 0;
};

