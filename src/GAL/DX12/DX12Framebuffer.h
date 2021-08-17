#pragma once

#include "DX12RenderDevice.h"
#include "DX12RenderPass.h"
#include "DX12Texture.h"
#include "GAL/Framebuffer.h"

namespace GAL
{
	class DX12Framebuffer final : public Framebuffer
	{
	public:
		DX12Framebuffer() = default;

		void Initialize(const DX12RenderDevice* renderDevice, DX12RenderPass renderPass, GTSL::Extent2D extent, GTSL::Range<const DX12TextureView*> textureViews) {
		}

		void Destroy(const DX12RenderDevice* renderDevice) {}

		~DX12Framebuffer() = default;

		[[nodiscard]] uint64_t GetHandle() const { return 0; }

	private:
	};
}
