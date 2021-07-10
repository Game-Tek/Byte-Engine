#pragma once
#include <GTSL/FixedVector.h>
#include <GTSL/RGB.h>
#include <GTSL/Math/Rotator.h>


#include "ByteEngine/Game/System.h"


class LightsRenderGroup : public System
{
public:
	MAKE_HANDLE(uint32, DirectionalLight)
	MAKE_HANDLE(uint32, PointLight)
	
	LightsRenderGroup() : System(u8"LightsRenderGroup") {}
	
	void Initialize(const InitializeInfo& initializeInfo) override {
		directionalLights.Initialize(8, GetPersistentAllocator());
		pointLights.Initialize(8, GetPersistentAllocator());
	}

	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	DirectionalLightHandle CreateDirectionalLight() {
		return DirectionalLightHandle(directionalLights.Emplace());
	}

	PointLightHandle CreatePointLight() {
		return PointLightHandle(pointLights.Emplace());
	}

	void SetRotation(const DirectionalLightHandle lightHandle, const GTSL::Rotator rotator) {
		directionalLights[lightHandle()].Rotation = rotator;
	}

	void SetColor(const DirectionalLightHandle lightHandle, const GTSL::RGBA color) {
		directionalLights[lightHandle()].Color = color;
	}

	void SetColor(const PointLightHandle lightHandle, const GTSL::RGBA color) {
		pointLights[lightHandle()].Color = color;
	}

	void SetRadius(const PointLightHandle lightHandle, const float32 size) {
		pointLights[lightHandle()].Radius = size;
	}

private:
	struct DirectionalLight {
		GTSL::RGBA Color;
		GTSL::Rotator Rotation;
	};
	GTSL::FixedVector<DirectionalLight, BE::PersistentAllocatorReference> directionalLights;

	struct PointLight {
		GTSL::RGBA Color;
		float32 Radius;
	};
	GTSL::FixedVector<PointLight, BE::PersistentAllocatorReference> pointLights;

public:
	[[nodiscard]] auto& GetDirectionalLights() const { return directionalLights; }
};
