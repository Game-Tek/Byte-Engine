#pragma once
#include "RenderCore.h"
#include "GTSL/RGB.h"

#undef MemoryBarrier

namespace GAL
{
	class Buffer;
	class Texture;

	class CommandList
	{
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
			AccessType SourceAccess, DestinationAccess;
		};

		struct BufferBarrier {
			const Buffer* Buffer; GTSL::uint32 Size;
			AccessType SourceAccess, DestinationAccess;
		};

		struct TextureBarrier {
			const Texture* Texture;
			TextureLayout CurrentLayout, TargetLayout;
			AccessType SourceAccess, DestinationAccess;
			FormatDescriptor Format;
		};

		enum class BarrierType : GTSL::uint8 {
			MEMORY, BUFFER, TEXTURE
		};

		struct BarrierData
		{
			BarrierData(const MemoryBarrier memoryBarrier) : Type(BarrierType::MEMORY), Memory(memoryBarrier) {}
			BarrierData(const BufferBarrier bufferBarrier) : Type(BarrierType::BUFFER), Buffer(bufferBarrier) {}
			BarrierData(const TextureBarrier textureBarrier) : Type(BarrierType::TEXTURE), Texture(textureBarrier) {}

			BarrierType Type;

			union {
				MemoryBarrier Memory;
				BufferBarrier Buffer;
				TextureBarrier Texture;
			};

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
