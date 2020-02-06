#pragma once

struct Extent3D;

namespace RAPI
{
	class RenderPass;
	struct RenderPassBeginInfo;
	struct DrawInfo;
	class ComputePipeline;
	class GraphicsPipeline;
	struct PushConstantsInfo;
	struct BindBindingsSet;
	class RenderMesh;

	class CommandBuffer
	{
	public:
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
		virtual void BindMesh(RenderMesh* _Mesh) = 0;

		//    BIND PIPELINE COMMANDS

		//Adds a BindBindingsSet to the command queue.
		virtual void BindBindingsSet(const BindBindingsSet& bindBindingsSet) = 0;
		virtual void UpdatePushConstant(const PushConstantsInfo& _PCI) = 0;
		//Adds a BindGraphicsPipeline command to the command queue.
		virtual void BindGraphicsPipeline(GraphicsPipeline* _GP) = 0;
		//Adds a BindComputePipeline to the command queue.
		virtual void BindComputePipeline(ComputePipeline* _CP) = 0;


		//  DRAW COMMANDS

		//Adds a DrawIndexed command to the command queue.
		virtual void DrawIndexed(const DrawInfo& _DrawInfo) = 0;

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
	};
}
