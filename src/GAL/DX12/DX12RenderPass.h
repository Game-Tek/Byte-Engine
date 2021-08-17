#pragma once

#include "GAL/RenderPass.h"

namespace GAL {
	class DX12RenderDevice;
	
	class DX12RenderPass final : public RenderPass {
		void Initialize(const DX12RenderDevice* renderDevice, GTSL::Range<const RenderPassTargetDescription*> renderPassAttachments,
			GTSL::Range<const SubPassDescriptor*> subPasses, const GTSL::Range<const SubPassDependency*> subPassDependencies) {
		}

		void Destroy(const DX12RenderDevice* renderDevice) {}
	};
}
