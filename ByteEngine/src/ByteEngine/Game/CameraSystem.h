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
	
	void AddCamera()
	{
		positionMatrices.EmplaceBack(1);
		rotationMatrices.EmplaceBack(1);
		fovs.EmplaceBack(45.0f);
	}
	
	CameraHandle AddCamera(const GTSL::Vector3 pos)
	{
		rotationMatrices.EmplaceBack(1);
		fovs.EmplaceBack(45.0f);
		auto index = positionMatrices.GetLength();
		positionMatrices.EmplaceBack(pos);
		return CameraHandle(index);
	}
	
	//ComponentReference AddCamera(const GTSL::Matrix4& matrix)
	//{
	//	return viewMatrices.EmplaceBack(matrix);
	//}

	void RemoveCamera(const CameraHandle reference)
	{
		positionMatrices.Pop(reference());
		rotationMatrices.Pop(reference());
		fovs.Pop(reference());
	}

	void SetCameraRotation(const CameraHandle reference, const GTSL::Matrix4 matrix4)
	{
		rotationMatrices[reference()] = matrix4;
	}
	
	void SetCameraPosition(const CameraHandle reference, const GTSL::Vector3 pos)
	{
		positionMatrices[reference()] = GTSL::Matrix4(pos);
	}

	void AddCameraPosition(const CameraHandle reference, GTSL::Vector3 pos)
	{
		GTSL::Math::Translate(positionMatrices[reference()], pos);
	}

	void AddCameraRotation(const CameraHandle reference, const GTSL::Quaternion quaternion)
	{
		GTSL::Math::Rotate(rotationMatrices[reference()], quaternion);
	}

	void AddCameraRotation(const CameraHandle reference, const GTSL::Matrix4 matrix)
	{
		rotationMatrices[reference()] = matrix * rotationMatrices[reference()];
	}
	
	[[nodiscard]] GTSL::Range<const GTSL::Matrix4*> GetPositionMatrices() const { return positionMatrices; }
	[[nodiscard]] GTSL::Range<const GTSL::Matrix4*> GetRotationMatrices() const { return rotationMatrices; }
	[[nodiscard]] GTSL::Range<const float32*> GetFieldOfViews() const { return fovs; }
	void SetFieldOfView(const CameraHandle componentReference, const float32 fov) { fovs[componentReference()] = fov; }
	float32 GetFieldOfView(const CameraHandle componentReference) const { return fovs[componentReference()]; }
	GTSL::Vector3 GetCameraPosition(CameraHandle cameraHandle) const { return GTSL::Math::GetTranslation(positionMatrices[cameraHandle()]); }

private:
	GTSL::Vector<GTSL::Matrix4, BE::PersistentAllocatorReference> positionMatrices;
	GTSL::Vector<GTSL::Matrix4, BE::PersistentAllocatorReference> rotationMatrices;
	GTSL::Vector<float32, BE::PersistentAllocatorReference> fovs;
};
