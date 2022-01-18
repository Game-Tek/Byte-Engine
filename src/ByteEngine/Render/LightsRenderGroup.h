#pragma once
#include <GTSL/FixedVector.hpp>
#include <GTSL/RGB.h>
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
		GetApplicationManager()->DispatchEvent(u8"LightsRenderGroup", EventHandle<PointLightHandle>(u8"OnAddPointLight"), GTSL::MoveRef(handle));
		return handle;
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

	void SetPosition(PointLightHandle point_light_handle, GTSL::Vector3 position) {
		GetApplicationManager()->DispatchEvent(u8"LightsRenderGroup", EventHandle<PointLightHandle, GTSL::Vector3>(u8"OnUpdatePointLight"), GTSL::MoveRef(point_light_handle), GTSL::MoveRef(position));
		pointLights[point_light_handle()].Position = position;
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
		GTSL::RGBA Color;
		float32 Radius;
		GTSL::Vector3 Position;
	};
	GTSL::FixedVector<PointLight, BE::PersistentAllocatorReference> pointLights;

public:
	[[nodiscard]] auto& GetDirectionalLights() const { return directionalLights; }
};
