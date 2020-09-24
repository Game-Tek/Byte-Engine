#pragma once
#include <GTSL/KeepVector.h>
#include <GTSL/RGB.h>
#include <GTSL/Math/Rotator.h>


#include "ByteEngine/Game/System.h"

class LightsRenderGroup : public System
{
public:
	struct RayTracingDirectionalLightCreateInfo
	{
		GTSL::RGBA Light;
		GTSL::Rotator Rotation;
	};
	ComponentReference CreateRayTracingDirectionalLight(const RayTracingDirectionalLightCreateInfo& info);
	
private:
	struct RayTracingDirectionalLight
	{
		GTSL::RGBA Light;
		GTSL::Rotator Rotation;
	};
	GTSL::KeepVector<RayTracingDirectionalLight, BE::PersistentAllocatorReference> rayTracingDirectionalLights;

public:
	[[nodiscard]] auto GetRayTracingDirectLights() const { return rayTracingDirectionalLights.GetRange(); }
};
