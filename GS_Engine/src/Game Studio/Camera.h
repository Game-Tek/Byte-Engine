#pragma once

#include "Core.h"

#include "Game/WorldObject.h"
#include "Containers/Pair.h"

class Camera : public WorldObject
{
	//First is near, Second is far
	using NearFarPair = Pair<float, float>;
public:
	Camera() = default;
	explicit Camera(const float FOV);
	~Camera() = default;

	[[nodiscard]] const char* GetName() const override { return "Camera"; }

	[[nodiscard]] float GetAperture() const { return Aperture; }
	[[nodiscard]] float GetIrisHeightMultiplier() const { return IrisHeightMultiplier; }
	[[nodiscard]] float& GetFOV() { return FOV; }
	[[nodiscard]] float GetFocusDistance() const { return FocusDistance; }
	[[nodiscard]] uint16 GetWhiteBalance() const { return WhiteBalance; }
	[[nodiscard]] uint16 GetISO() const { return ISO; }
	[[nodiscard]] const NearFarPair& GetNearFarPair() const { return nearFar; }

	void SetAperture(const float NewAperture) { Aperture = NewAperture; }

	void SetIrisHeightMultiplier(const float NewIrisHeightMultiplier)
	{
		IrisHeightMultiplier = NewIrisHeightMultiplier;
	}

	void SetFOV(const float NewFOV) { FOV = NewFOV; }
	void SetFocusDistance(const float NewFocusDistance) { FocusDistance = NewFocusDistance; }
	void SetFocusDistance(const Vector3& Object);
	void SetWhiteBalance(const uint16 NewWhiteBalance) { WhiteBalance = NewWhiteBalance; }
	void SetISO(const uint16 NewISO) { ISO = NewISO; }
	void SetNearFar(const NearFarPair& _NFP) { nearFar = _NFP; }

protected:
	float FOV = 45.0f;
	float FocusDistance = 0.0f;

	float Aperture = 2.8f;
	float IrisHeightMultiplier = 1.0f;

	uint16 WhiteBalance = 4000;
	uint16 ISO = 1800;

	NearFarPair nearFar = NearFarPair(1.0f, 1000.0f);
};
