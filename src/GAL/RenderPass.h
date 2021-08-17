#pragma once

#include "RenderCore.h"

namespace GAL
{
	constexpr GTSL::uint8 ATTACHMENT_UNUSED = 255;

	class RenderPass {
	public:
		RenderPass() = default;

		//Describes the reference to a render pass attachment for a sub pass.
		struct AttachmentReference
		{
			GTSL::uint8 Index = ATTACHMENT_UNUSED;
			//Layout of the attachment during the sub pass.
			TextureLayout Layout = TextureLayout::ATTACHMENT;
			AccessType Access;
		};

		//Describes a subpass.
		struct SubPassDescriptor
		{
			//Array of AttachmentsReferences
			GTSL::Range<const AttachmentReference*> Attachments;

			//Array of indices identifying attachments that are not used by this subpass, but whose contents MUST be preserved throughout the subpass.
			GTSL::Range<const GTSL::uint8*> PreserveAttachments;
		};
		
		static constexpr GTSL::uint8 EXTERNAL = 255;

		struct SubPassDependency
		{
			GTSL::uint8 SourceSubPass, DestinationSubPass;
			PipelineStage SourcePipelineStage, DestinationPipelineStage;
			AccessType SourceAccessType, DestinationAccessType;
		};
		
		~RenderPass() = default;
	};

}
