#pragma once

#include "Core.h"

#include "Game/WorldObject.h"
#include <GTSL/Pair.h>

class Camera : public WorldObject
{
	//First is near, Second is far
	using NearFarPair = GTSL::Pair<float, float>;
public:
	Camera() = default;
	explicit Camera(const float FOV);
	~Camera() = default;

	void Destroy(World* ownerWorld) override
	{}
	
	[[nodiscard]] const char* GetName() const override { return "Camera"; }

	[[nodiscard]] float GetAperture() const { return aperture; }
	[[nodiscard]] float GetIrisHeightMultiplier() const { return irisHeightMultiplier; }
	[[nodiscard]] float& GetFOV() { return FOV; }
	[[nodiscard]] float GetFocusDistance() const { return focusDistance; }
	[[nodiscard]] uint16 GetWhiteBalance() const { return whiteBalance; }
	[[nodiscard]] uint16 GetISO() const { return ISO; }
	[[nodiscard]] const NearFarPair& GetNearFarPair() const { return nearFar; }

	void SetAperture(const float NewAperture) { aperture = NewAperture; }

	void SetIrisHeightMultiplier(const float NewIrisHeightMultiplier)
	{
		irisHeightMultiplier = NewIrisHeightMultiplier;
	}

	void SetFOV(const float NewFOV) { FOV = NewFOV; }
	void SetFocusDistance(const float NewFocusDistance) { focusDistance = NewFocusDistance; }
	void SetFocusDistance(const GTSL::Vector3& Object);
	void SetWhiteBalance(const uint16 NewWhiteBalance) { whiteBalance = NewWhiteBalance; }
	void SetISO(const uint16 NewISO) { ISO = NewISO; }
	void SetNearFar(const NearFarPair& _NFP) { nearFar = _NFP; }

protected:
	float FOV = 45.0f;
	float focusDistance = 0.0f;

	float aperture = 2.8f;
	float irisHeightMultiplier = 1.0f;

	uint16 whiteBalance = 4000;
	uint16 ISO = 1800;

	NearFarPair nearFar = NearFarPair(1.0f, 1000.0f);
};
