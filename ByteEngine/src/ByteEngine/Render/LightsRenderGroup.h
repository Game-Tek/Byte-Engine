#pragma once
#include <GTSL/KeepVector.h>
#include <GTSL/RGB.h>
#include <GTSL/Math/Rotator.h>


#include "ByteEngine/Game/System.h"

MAKE_HANDLE(uint32, DirectionalLight)

class LightsRenderGroup : public System
{
public:
	LightsRenderGroup() : System("LightsRenderGroup") {}
	
	void Initialize(const InitializeInfo& initializeInfo) override
	{
		directionalLights.Initialize(8, GetPersistentAllocator());
	}

	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	DirectionalLightHandle CreateDirectionalLight()
	{
		return DirectionalLightHandle(directionalLights.Emplace());
	}

	void SetLightRotation(const DirectionalLightHandle lightHandle, const GTSL::Rotator rotator)
	{
		directionalLights[lightHandle()].Rotation = rotator;
	}

	void SetLightColor(const DirectionalLightHandle lightHandle, const GTSL::RGBA color)
	{
		directionalLights[lightHandle()].Color = color;
	}

private:
	struct DirectionalLight
	{
		GTSL::RGBA Color;
		GTSL::Rotator Rotation;
	};
	GTSL::KeepVector<DirectionalLight, BE::PersistentAllocatorReference> directionalLights;

public:
	[[nodiscard]] auto GetDirectionalLights() const { return directionalLights.GetRange(); }
};
