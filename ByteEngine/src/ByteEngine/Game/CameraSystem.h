#pragma once

#include "ByteEngine/Core.h"
#include "ByteEngine/Game/System.h"

#include <GTSL/Math/Matrix4.h>
#include <GTSL/Math/Math.hpp>
#include <GTSL/Vector.hpp>

class CameraSystem : public System
{
public:
	CameraSystem() : positionMatrices(4, GetPersistentAllocator()), rotationMatrices(4, GetPersistentAllocator()), fovs(4, GetPersistentAllocator())
	{}

	void Initialize(const InitializeInfo& initializeInfo) override {}
	void Shutdown(const ShutdownInfo& shutdownInfo) override {}
	
	void AddCamera()
	{
		positionMatrices.EmplaceBack(1);
		rotationMatrices.EmplaceBack(1);
		fovs.EmplaceBack(45.0f);
	}
	
	ComponentReference AddCamera(const GTSL::Vector3 pos)
	{
		rotationMatrices.EmplaceBack(1);
		fovs.EmplaceBack(45.0f);
		return ComponentReference(GetSystemId(), positionMatrices.EmplaceBack(GTSL::Math::Translation(pos)));
	}
	
	//ComponentReference AddCamera(const GTSL::Matrix4& matrix)
	//{
	//	return viewMatrices.EmplaceBack(matrix);
	//}

	void RemoveCamera(const ComponentReference reference)
	{
		positionMatrices.Pop(reference.Component);
		rotationMatrices.Pop(reference.Component);
		fovs.Pop(reference.Component);
	}

	void SetCameraRotation(const ComponentReference reference, const GTSL::Matrix4 matrix4)
	{
		rotationMatrices[reference.Component] = matrix4;
	}
	
	void SetCameraPosition(const ComponentReference reference, const GTSL::Vector3 pos)
	{
		positionMatrices[reference.Component] = GTSL::Math::Translation(pos);
	}

	void AddCameraPosition(const ComponentReference reference, GTSL::Vector3 pos)
	{
		GTSL::Math::Translate(positionMatrices[reference.Component], pos);
	}

	void AddCameraRotation(const ComponentReference reference, const GTSL::Quaternion quaternion)
	{
		GTSL::Math::Rotate(rotationMatrices[reference.Component], quaternion);
	}

	void AddCameraRotation(const ComponentReference reference, const GTSL::Matrix4 matrix)
	{
		rotationMatrices[reference.Component] = matrix * rotationMatrices[reference.Component];
	}
	
	[[nodiscard]] GTSL::Range<const GTSL::Matrix4*> GetPositionMatrices() const { return positionMatrices; }
	[[nodiscard]] GTSL::Range<const GTSL::Matrix4*> GetRotationMatrices() const { return rotationMatrices; }
	[[nodiscard]] GTSL::Range<const float32*> GetFieldOfViews() const { return fovs; }
	void SetFieldOfView(const ComponentReference componentReference, const float32 fov) { fovs[componentReference.Component] = fov; }

private:
	GTSL::Vector<GTSL::Matrix4, BE::PersistentAllocatorReference> positionMatrices;
	GTSL::Vector<GTSL::Matrix4, BE::PersistentAllocatorReference> rotationMatrices;
	GTSL::Vector<float32, BE::PersistentAllocatorReference> fovs;
};
