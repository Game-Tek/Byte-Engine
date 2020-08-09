#pragma once

#include "ComponentCollection.h"

#include <GTSL/Math/Matrix4.h>
#include <GTSL/Math/Math.hpp>
#include <GTSL/Vector.hpp>

class CameraComponentCollection : public ComponentCollection
{
public:
	CameraComponentCollection() : positionMatrices(4, GetPersistentAllocator()), rotationMatrices(4, GetPersistentAllocator())
	{}

	void AddCamera()
	{
		positionMatrices.EmplaceBack(1);
		rotationMatrices.EmplaceBack(1);
	}
	
	ComponentReference AddCamera(const GTSL::Vector3 pos)
	{
		rotationMatrices.EmplaceBack(1);
		return positionMatrices.EmplaceBack(GTSL::Math::Translation(pos));
	}
	
	//ComponentReference AddCamera(const GTSL::Matrix4& matrix)
	//{
	//	return viewMatrices.EmplaceBack(matrix);
	//}

	void RemoveCamera(const ComponentReference reference)
	{
		positionMatrices.Pop(reference);
		rotationMatrices.Pop(reference);
	}

	void SetCameraRotation(const ComponentReference reference, const GTSL::Matrix4 matrix4)
	{
		rotationMatrices[reference] = matrix4;
	}
	
	void SetCameraPosition(const ComponentReference reference, const GTSL::Vector3 pos)
	{
		positionMatrices[reference] = GTSL::Math::Translation(pos);
	}

	void AddCameraPosition(const ComponentReference reference, GTSL::Vector3 pos)
	{
		GTSL::Math::Translate(positionMatrices[reference], pos);
	}

	void AddCameraRotation(const ComponentReference reference, const GTSL::Quaternion quaternion)
	{
		GTSL::Math::Rotate(rotationMatrices[reference], quaternion);
	}

	void AddCameraRotation(const ComponentReference reference, const GTSL::Matrix4 matrix)
	{
		rotationMatrices[reference] = matrix * rotationMatrices[reference];
	}
	
	[[nodiscard]] GTSL::Ranger<const GTSL::Matrix4> GetPositionMatrices() const { return positionMatrices; }
	[[nodiscard]] GTSL::Ranger<const GTSL::Matrix4> GetRotationMatrices() const { return rotationMatrices; }
	
private:
	GTSL::Vector<GTSL::Matrix4, BE::PersistentAllocatorReference> positionMatrices;
	GTSL::Vector<GTSL::Matrix4, BE::PersistentAllocatorReference> rotationMatrices;
};
