#pragma once

#include "DX12.h"
#include <dxgi1_6.h>

#include "DX12Queue.h"
#include "GAL/RenderContext.h"
#include "GAL/RenderCore.h"
#include "GTSL/Extent.h"
#include "GAL/DX12/DX12RenderDevice.h"
#include <wrl.h>

#include "GTSL/Application.h"
#include "GTSL/Window.h"

namespace GAL
{
	class DX12Queue;

	class DX12Surface
	{
	public:
		void Initialize(const DX12RenderDevice* render_device, const GTSL::Application& application, const GTSL::Window& window) {
			handle = window.GetHWND();
		}

		[[nodiscard]] HWND GetHWND() const { return handle; }
	private:
		HWND handle = nullptr;
	};

	class DX12RenderContext
	{
	public:
		DX12RenderContext() = default;

		bool InitializeOrRecreate(const DX12RenderDevice* renderDevice, const DX12Queue queue, const DX12Surface surface, GTSL::Extent2D extent, FormatDescriptor format, ColorSpace colorSpace,
			TextureUse textureUse, PresentModes presentMode, GTSL::uint8 desiredFramesInFlight) {
			
			if(swapChain4) {				
				return swapChain4->ResizeBuffers(desiredFramesInFlight, extent.Width, extent.Height, ToDX12(format), tear ? DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING : 0) == S_OK;
			}

			IDXGIFactory4* factory4; GTSL::uint32 factoryFlags = 0;

			if constexpr (_DEBUG) {
				factoryFlags |= DXGI_CREATE_FACTORY_DEBUG;
			}

			DX_CHECK(CreateDXGIFactory2(factoryFlags, __uuidof(IDXGIFactory4), reinterpret_cast<void**>(&factory4)));

			BOOL allowTearing = false;

			{
				IDXGIFactory5* factory5;

				if (factory4->QueryInterface(__uuidof(IDXGIFactory5), reinterpret_cast<void**>(&factory5)) >= 0) {
					factory5->CheckFeatureSupport(DXGI_FEATURE_PRESENT_ALLOW_TEARING, &allowTearing, sizeof(allowTearing));
				}

				factory5->Release();
			}

			vSync = presentMode == PresentModes::SWAP ? true : false;
			tear = static_cast<bool>(allowTearing);
			
			DXGI_SWAP_CHAIN_DESC1 swapChainDesc{};
			swapChainDesc.Width = extent.Width;
			swapChainDesc.Height = extent.Height;
			swapChainDesc.Format = ToDX12(format);
			swapChainDesc.Stereo = FALSE;
			swapChainDesc.SampleDesc = { 1, 0 };
			swapChainDesc.BufferUsage = DXGI_USAGE_RENDER_TARGET_OUTPUT;
			swapChainDesc.BufferCount = desiredFramesInFlight;
			swapChainDesc.Scaling = DXGI_SCALING_STRETCH;
			swapChainDesc.SwapEffect = DXGI_SWAP_EFFECT_FLIP_DISCARD;
			swapChainDesc.AlphaMode = DXGI_ALPHA_MODE_UNSPECIFIED;
			// It is recommended to always allow tearing if tearing support is available.
			swapChainDesc.Flags |= tear ? DXGI_SWAP_CHAIN_FLAG_ALLOW_TEARING : 0;

			IDXGISwapChain1* swapChain1;

			DX_CHECK(factory4->CreateSwapChainForHwnd(queue.GetID3D12CommandQueue(), static_cast<HWND>(surface.GetHWND()), &swapChainDesc, nullptr, nullptr, &swapChain1))

			DX_CHECK(factory4->MakeWindowAssociation(static_cast<HWND>(surface.GetHWND()), DXGI_MWA_NO_ALT_ENTER))

			DX_CHECK(swapChain1->QueryInterface(__uuidof(IDXGISwapChain4), reinterpret_cast<void**>(&swapChain4)))

			swapChain1->Release();

			factory4->Release();

			return true;
		}

		void Destroy(const DX12RenderDevice* renderDevice) {
			swapChain4->Release();
			debugClear(swapChain4);
		}

		//When you call IDXGISwapChain1::Present1 on a full - screen application, the swap chain flips(as opposed to blits) the contents of the back buffer
		//to the front buffer.This requires that the swap chain was created by using an enumerated display mode(specified in DXGI_SWAP_CHAIN_DESC1).
		//If you fail to enumerate display modes, or incorrectly specify the display mode in the description, the swap chain may perform a bit - block transfer(bitblt) instead.
		//The bitblt causes an extra stretching copy as well as some increased video memory usage, and is difficult to detect.To avoid this problem, enumerate display modes,
		//and initialize the swap chain description correctly before you create the swap chain.This will ensure maximum performance when flipping in full - screen mode
		//and avoid the extra memory overhead.
		void Present(const DX12RenderDevice* renderDevice, GTSL::Range<const GTSL::uint32*> waitSemaphores, GTSL::uint32 imageIndex, DX12Queue queue) {
			UINT flags = 0; flags |= tear ? DXGI_PRESENT_ALLOW_TEARING : 0;

			RECT rect;

			HWND hwnd;
			swapChain4->GetHwnd(&hwnd);

			GetClientRect(hwnd, &rect);

			DXGI_PRESENT_PARAMETERS presentParameters;
			presentParameters.DirtyRectsCount = 1;
			presentParameters.pDirtyRects = &rect;
			presentParameters.pScrollOffset = nullptr;
			presentParameters.pScrollRect = nullptr;

			swapChain4->Present1(vSync && !tear, flags, &presentParameters);
		}
	private:
		IDXGISwapChain4* swapChain4 = nullptr; bool vSync = false; bool tear = false;
	};
}
