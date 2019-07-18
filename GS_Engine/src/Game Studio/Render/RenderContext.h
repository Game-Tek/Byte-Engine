#pragma once

#include "Core.h"
#include "Extent.h"

class Window;
class Mesh;
class VertexBuffer;
class IndexBuffer;
class RenderPass;
class GraphicsPipeline;
class ComputePipeline;
class Framebuffer;

GS_STRUCT CommandBufferCreateInfo
{

};

GS_STRUCT DrawInfo
{
	uint16 IndexCount = 0;
	uint16 InstanceCount = 1;
};

GS_STRUCT RenderPassBeginInfo
{
	RenderPass * RenderPass = nullptr;
	Framebuffer* Framebuffer = nullptr;
	Extent2D RenderArea;
};

GS_STRUCT RenderContextCreateInfo
{
	Extent2D Extent;
	Window* Window = nullptr;
};

GS_CLASS RenderContext
{
public:

	virtual void OnResize() = 0;

	virtual void Present() = 0;

	//Starts recording of commands.
	virtual void BeginRecording() = 0;
	//Ends recording of commands.
	virtual void EndRecording() = 0;
	
	//Sends all commands to the GPU.
	virtual void Flush() = 0;
	
	
	// COMMANDS
	
	//  BIND COMMANDS
	//    BIND BUFFER COMMANDS
	
	
	//Adds a BindVertexBuffer command to the buffer.
	virtual void BindVertexBuffer(VertexBuffer* _VB) = 0;
	//Adds a BindIndexBuffer command to the buffer.
	virtual void BindIndexBuffer(IndexBuffer* _IB) = 0;
	
	//    BIND PIPELINE COMMANDS
	
	//Adds a BindGraphicsPipeline command to the buffer.
	virtual void BindGraphicsPipeline(GraphicsPipeline* _GP) = 0;
	//Adds a BindComputePipeline to the buffer.
	virtual void BindComputePipeline(ComputePipeline* _CP) = 0;
	
	
	//  DRAW COMMANDS
	
	//Adds a DrawIndexed command to the buffer.
	virtual void DrawIndexed(const DrawInfo& _DI) = 0;
	//Adds a DrawIndexedInstanced command to the buffer.
	virtual void DrawIndexedInstanced(uint16 _IndexCount) = 0;
	
	//  COMPUTE COMMANDS
	
	//Adds a Dispatch command to the buffer.
	virtual void Dispatch(uint32 _WorkGroupsX, uint32 _WorkGroupsY, uint32 _WorkGroupsZ) = 0;
	
	//  RENDER PASS COMMANDS
	
	//Adds a BeginRenderPass command to the buffer.
	virtual void BeginRenderPass(const RenderPassBeginInfo& _RPBI) = 0;
	//Adds a EndRenderPass command to the buffer.
	virtual void EndRenderPass(RenderPass* _RP) = 0;
};