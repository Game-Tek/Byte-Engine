#pragma once
#include <GTSL/FixedVector.hpp>
#include <GTSL/RGB.hpp>
#include <GTSL/Math/Rotator.h>

#include "ByteEngine/Game/System.hpp"

class LightsRenderGroup : public BE::System {
public:
	MAKE_HANDLE(uint32, DirectionalLight)
	MAKE_HANDLE(uint32, PointLight)
	
	LightsRenderGroup(const InitializeInfo& initializeInfo) : System(initializeInfo, u8"LightsRenderGroup"), directionalLights(8, GetPersistentAllocator()), pointLights(16, GetPersistentAllocator())
	{
	}
	
	DirectionalLightHandle CreateDirectionalLight() {
		return DirectionalLightHandle(directionalLights.Emplace());
	}

	PointLightHandle CreatePointLight() {
		auto handle = PointLightHandle(pointLights.Emplace());
		auto& light = pointLights[handle()];
		light.Lumens = 1.0f; light.Color.R() = 1.0f; light.Color.G() = 1.0f; light.Color.B() = 1.0f;
		light.Radius = 0.5f;
		GetApplicationManager()->DispatchEvent(this, EventHandle<PointLightHandle>(u8"OnAddPointLight"), GTSL::MoveRef(handle));
		return handle;
	}

	void SetRotation(const DirectionalLightHandle lightHandle, const GTSL::Rotator rotator) {
		directionalLights[lightHandle()].Rotation = rotator;
	}

	void SetColor(const DirectionalLightHandle lightHandle, const GTSL::RGBA color) {
		directionalLights[lightHandle()].Color = color;
	}

	void SetColor(PointLightHandle point_light_handle, const GTSL::RGB color) {
		auto& light = pointLights[point_light_handle()];
		light.Color = color;
		GetApplicationManager()->DispatchEvent(this, EventHandle<PointLightHandle, GTSL::Vector3, GTSL::RGB, float32, float32>(u8"OnUpdatePointLight"), GTSL::MoveRef(point_light_handle), GTSL::MoveRef(light.Position), GTSL::MoveRef(light.Color), GTSL::MoveRef(light.Lumens), GTSL::MoveRef(light.Radius));
	}

	void SetLumens(PointLightHandle point_light_handle, const float32 lumens) {
		auto& light = pointLights[point_light_handle()];
		light.Lumens = lumens;
		GetApplicationManager()->DispatchEvent(this, EventHandle<PointLightHandle, GTSL::Vector3, GTSL::RGB, float32, float32>(u8"OnUpdatePointLight"), GTSL::MoveRef(point_light_handle), GTSL::MoveRef(light.Position), GTSL::MoveRef(light.Color), GTSL::MoveRef(light.Lumens), GTSL::MoveRef(light.Radius));
	}

	void SetPosition(PointLightHandle point_light_handle, GTSL::Vector3 position) {
		auto& light = pointLights[point_light_handle()];
		light.Position = position;
		GetApplicationManager()->DispatchEvent(this, EventHandle<PointLightHandle, GTSL::Vector3, GTSL::RGB, float32, float32>(u8"OnUpdatePointLight"), GTSL::MoveRef(point_light_handle), GTSL::MoveRef(light.Position), GTSL::MoveRef(light.Color), GTSL::MoveRef(light.Lumens), GTSL::MoveRef(light.Radius));
	}

	void SetRadius(PointLightHandle point_light_handle, const float32 radius) {
		auto& light = pointLights[point_light_handle()];
		light.Radius = radius;
		GetApplicationManager()->DispatchEvent(this, EventHandle<PointLightHandle, GTSL::Vector3, GTSL::RGB, float32, float32>(u8"OnUpdatePointLight"), GTSL::MoveRef(point_light_handle), GTSL::MoveRef(light.Position), GTSL::MoveRef(light.Color), GTSL::MoveRef(light.Lumens), GTSL::MoveRef(light.Radius));
	}

	GTSL::Vector3 GetPosition(const PointLightHandle point_light_handle) const {
		return pointLights[point_light_handle()].Position;
	}

private:
	struct DirectionalLight {
		GTSL::RGBA Color;
		GTSL::Rotator Rotation;
	};
	GTSL::FixedVector<DirectionalLight, BE::PersistentAllocatorReference> directionalLights;

	struct PointLight {
		GTSL::RGB Color;
		float32 Lumens;
		GTSL::Vector3 Position;
		float32 Radius;
	};
	GTSL::FixedVector<PointLight, BE::PersistentAllocatorReference> pointLights;

public:
	[[nodiscard]] auto& GetDirectionalLights() const { return directionalLights; }
};
