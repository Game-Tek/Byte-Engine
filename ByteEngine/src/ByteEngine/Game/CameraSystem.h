#pragma once

#include "ByteEngine/Core.h"
#include "ByteEngine/Handle.hpp"
#include "ByteEngine/Game/System.h"

#include <GTSL/Math/Matrix4.h>
#include <GTSL/Math/Math.hpp>
#include <GTSL/Vector.hpp>


class CameraSystem : public System
{
public:
	CameraSystem() : positionMatrices(4, GetPersistentAllocator()), rotationMatrices(4, GetPersistentAllocator()), fovs(4, GetPersistentAllocator())
	{}

	MAKE_HANDLE(uint32, Camera);
	
	void Initialize(const InitializeInfo& initializeInfo) override {}
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	CameraHandle AddCamera(const GTSL::Vector3 pos)
	{
		rotationMatrices.EmplaceBack();
		fovs.EmplaceBack(45.0f);
		auto index = positionMatrices.GetLength();
		positionMatrices.EmplaceBack(pos);
		return CameraHandle(index);
	}

	void RemoveCamera(const CameraHandle reference)
	{
		positionMatrices.Pop(reference());
		rotationMatrices.Pop(reference());
		fovs.Pop(reference());
	}

	void SetCameraRotation(const CameraHandle reference, const GTSL::Quaternion quaternion) {
		rotationMatrices[reference()] = GTSL::Matrix4(quaternion);
	}

	void SetCameraRotation(const CameraHandle reference, const GTSL::Matrix4 matrix4) {
		rotationMatrices[reference()] = matrix4;
	}

	GTSL::Matrix4 GetCameraTransform() const {
		auto pos = positionMatrices[0];
		pos[0][3] *= -1;
		pos[1][3] *= -1;
		pos[2][3] *= -1;
		
		return rotationMatrices[0] * pos;
	}
	
	void SetCameraPosition(const CameraHandle reference, const GTSL::Vector3 pos) {
		GTSL::Math::SetTranslation(positionMatrices[reference()], pos);
	}

	void AddCameraPosition(const CameraHandle reference, GTSL::Vector3 pos) {
		GTSL::Math::AddTranslation(positionMatrices[reference()], pos);
	}

	void AddCameraRotation(const CameraHandle reference, const GTSL::Quaternion quaternion)
	{
		rotationMatrices[reference()] *= GTSL::Matrix4(quaternion);
	}

	void AddCameraRotation(const CameraHandle reference, const GTSL::Matrix4 matrix4)
	{
		rotationMatrices[reference()] *= matrix4;
	}
	
	[[nodiscard]] GTSL::Range<const float32*> GetFieldOfViews() const { return fovs; }
	void SetFieldOfView(const CameraHandle componentReference, const float32 fov) { fovs[componentReference()] = fov; }
	float32 GetFieldOfView(const CameraHandle componentReference) const { return fovs[componentReference()]; }
	GTSL::Vector3 GetCameraPosition(CameraHandle cameraHandle) const { return GTSL::Math::GetTranslation(positionMatrices[cameraHandle()]); }
	//GTSL::Quaternion GetCameraOrientation(CameraHandle cameraHandle) const { return rotationMatrices[cameraHandle()]; }

private:
	GTSL::Vector<GTSL::Matrix4, BE::PersistentAllocatorReference> positionMatrices;
	GTSL::Vector<GTSL::Matrix4, BE::PersistentAllocatorReference> rotationMatrices;
	GTSL::Vector<float32, BE::PersistentAllocatorReference> fovs;
};
