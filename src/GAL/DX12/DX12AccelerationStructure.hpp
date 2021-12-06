#pragma once

#include "DX12.h"
#include <GAL/DX12/DX12RenderDevice.h>

namespace GAL
{
	class DX12AccelerationStructure
	{
	public:
		DX12AccelerationStructure(const DX12RenderDevice* renderDevice) {
			//renderDevice->GetID3D12Device2()->CreateAcc
			ID3D12Device8* d;
		}
	private:
		
	};
}
