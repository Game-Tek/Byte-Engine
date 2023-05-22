#pragma once
#include "RenderCore.h"
#include <GTSL/RGB.hpp>

#undef MemoryBarrier

namespace GAL
{
	class Buffer;
	class Texture;

	class CommandList {
	public:
		CommandList() = default;
		~CommandList() = default;
		//Starts recording of commands.

		
		//Ends recording of commands.

		// COMMANDS

		//  BIND COMMANDS
		//    BIND BUFFER COMMANDS

		//Adds a BindMesh command to the command queue.
		
		//    BIND PIPELINE COMMANDS

		//Adds a BindBindingsSets to the command queue.

		//Adds a BindGraphicsPipeline command to the command queue.
		//Adds a BindComputePipeline to the command queue.

		//  DRAW COMMANDS

		//Adds a DrawIndexed command to the command queue.

		//  COMPUTE COMMANDS

		//Adds a Dispatch command to the command queue.

		//  RENDER PASS COMMANDS

		//Adds a BeginRenderPass command to the command queue.

		//Adds a AdvanceSubPass command to the command buffer.
		
		struct MemoryBarrier {
		};

		struct BufferBarrier {
			const Buffer* buffer; GTSL::uint32 size;
		};

		struct TextureBarrier {
			const Texture* texture;
			TextureLayout CurrentLayout, TargetLayout;
			FormatDescriptor Format;
		};

		enum class BarrierType : GTSL::uint8 {
			MEMORY, BUFFER, TEXTURE
		};

		struct BarrierData {
			BarrierData(PipelineStage sP, PipelineStage dP, AccessType sA, AccessType dA, const MemoryBarrier memoryBarrier)   : SourceStage(sP), DestinationStage(dP), SourceAccess(sA), DestinationAccess(dA), Type(BarrierType::MEMORY), Memory(memoryBarrier) {}
			BarrierData(PipelineStage sP, PipelineStage dP, AccessType sA, AccessType dA, const BufferBarrier bufferBarrier)   : SourceStage(sP), DestinationStage(dP), SourceAccess(sA), DestinationAccess(dA), Type(BarrierType::BUFFER), Buffer(bufferBarrier) {}
			BarrierData(PipelineStage sP, PipelineStage dP, AccessType sA, AccessType dA, const TextureBarrier textureBarrier) : SourceStage(sP), DestinationStage(dP), SourceAccess(sA), DestinationAccess(dA), Type(BarrierType::TEXTURE),Texture(textureBarrier) {}

			BarrierData(const BarrierData& other) : Type(other.Type) {
				switch (Type) {
				case BarrierType::MEMORY: Memory = other.Memory; break;
				case BarrierType::BUFFER: Buffer = other.Buffer; break;
				case BarrierType::TEXTURE: Texture = other.Texture; break;
				}
			}

			BarrierType Type;

			union {
				MemoryBarrier Memory;
				BufferBarrier Buffer;
				TextureBarrier Texture;
			};

			AccessType SourceAccess, DestinationAccess;
			PipelineStage SourceStage, DestinationStage;

			GTSL::uint32 From = 0xFFFFFFFF, To = 0xFFFFFFFF;

			void SetMemoryBarrier(MemoryBarrier memoryBarrier) { Type = BarrierType::MEMORY; Memory = memoryBarrier; }
			void SetTextureBarrier(TextureBarrier textureBarrier) { Type = BarrierType::TEXTURE; Texture = textureBarrier; }
			void SetBufferBarrier(BufferBarrier bufferBarrier) { Type = BarrierType::BUFFER; Buffer = bufferBarrier; }
		};

		struct ShaderTableDescriptor
		{
			DeviceAddress Address;

			/**
			 * \brief Number of entries in the shader group.
			 */
			GTSL::uint32 Entries = 0;

			/**
			 * \brief Size of each entry in the shader group.
			 */
			GTSL::uint32 EntrySize = 0;
		};
	};
}
