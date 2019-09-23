#pragma once

#include "Core.h"
#include "Utility/Extent.h"
#include "Image.h"
#include "Containers/FVector.hpp"

class UniformLayout;
class Window;
class Mesh;
class VertexBuffer;
class IndexBuffer;
class RenderPass;
class GraphicsPipeline;
class ComputePipeline;
class Framebuffer;

struct GS_API DrawInfo
{
	uint16 IndexCount = 0;
	uint16 InstanceCount = 1;
};

struct GS_API RenderPassBeginInfo
{
	RenderPass* RenderPass = nullptr;
	Framebuffer** Framebuffers = nullptr;
};

struct GS_API PushConstantsInfo
{
	UniformLayout* UniformLayout = nullptr;
	uint32 Offset = 0;
	uint32 Size = 0;
	void* Data = nullptr;
};

struct GS_API RenderContextCreateInfo
{
	Window* Window = nullptr;
};

class GS_API RenderContext
{
protected:
	uint8 CurrentImage = 0;
	uint8 MAX_FRAMES_IN_FLIGHT = 0;

public:
	virtual ~RenderContext() {};

	virtual void OnResize() = 0;

	//Starts recording of commands.
	virtual void BeginRecording() = 0;
	//Ends recording of commands.
	virtual void EndRecording() = 0;

	virtual void AcquireNextImage() = 0;

	//Sends all commands to the GPU.
	virtual void Flush() = 0;

	//Swaps buffers and sends new image to the screen.
	virtual void Present() = 0;
	
	// COMMANDS
	
	//  BIND COMMANDS
	//    BIND BUFFER COMMANDS
	
	
	//Adds a BindMesh command to the command queue.
	virtual void BindMesh(Mesh* _Mesh) = 0;
	
	//    BIND PIPELINE COMMANDS

	//Adds a BindUniformLayout to the command queue.
	virtual void BindUniformLayout(UniformLayout* _UL) = 0;
	virtual void UpdatePushConstant(const PushConstantsInfo& _PCI) = 0;
	//Adds a BindGraphicsPipeline command to the command queue.
	virtual void BindGraphicsPipeline(GraphicsPipeline* _GP) = 0;
	//Adds a BindComputePipeline to the command queue.
	virtual void BindComputePipeline(ComputePipeline* _CP) = 0;
	
	
	//  DRAW COMMANDS
	
	//Adds a DrawIndexed command to the command queue.
	virtual void DrawIndexed(const DrawInfo& _DI) = 0;
	
	//  COMPUTE COMMANDS
	
	//Adds a Dispatch command to the command queue.
	virtual void Dispatch(const Extent3D& _WorkGroups) = 0;

	//  RENDER PASS COMMANDS
	
	//Adds a BeginRenderPass command to the command queue.
	virtual void BeginRenderPass(const RenderPassBeginInfo& _RPBI) = 0;
	//Adds a AdvanceSubPass command to the command buffer.
	virtual void AdvanceSubPass() = 0;
	//Adds a EndRenderPass command to the command queue.
	virtual void EndRenderPass(RenderPass* _RP) = 0;

	[[nodiscard]] virtual FVector<Image*> GetSwapchainImages() const = 0;

	[[nodiscard]] uint8 GetCurrentImage() const { return CurrentImage; }
	[[nodiscard]] uint8 GetMaxFramesInFlight() const { return MAX_FRAMES_IN_FLIGHT; }
};