#pragma once

#include "ComponentCollection.h"

#include <GTSL/Math/Matrix4.h>
#include <GTSL/Math/Math.hpp>
#include <GTSL/Vector.hpp>

class CameraComponentCollection : public ComponentCollection
{
public:
	CameraComponentCollection() : viewMatrices(4, GetPersistentAllocator())
	{}

	void AddCamera() { viewMatrices.EmplaceBack(); }
	ComponentReference AddCamera(const GTSL::Vector3 pos) { return viewMatrices.EmplaceBack(GTSL::Math::Translation(pos)); }
	ComponentReference AddCamera(const GTSL::Matrix4& matrix) { return viewMatrices.EmplaceBack(matrix); }

	void RemoveCamera(const ComponentReference reference) { viewMatrices.Pop(reference); }
	
	void SetCameraPosition(const ComponentReference reference, const GTSL::Vector3 pos)
	{
		viewMatrices[reference] = GTSL::Math::Translation(pos);
	}

	void AddCameraPosition(const ComponentReference reference, const GTSL::Vector3 pos)
	{
		GTSL::Math::Translate(viewMatrices[reference], pos);
	}

	void AddCameraRotation(const ComponentReference reference, const GTSL::Quaternion quaternion)
	{
		GTSL::Math::Rotate(viewMatrices[reference], quaternion);
	}
	
	[[nodiscard]] GTSL::Ranger<const GTSL::Matrix4> GetViewMatrices() const { return viewMatrices; }
	
private:
	GTSL::Vector<GTSL::Matrix4, BE::PersistentAllocatorReference> viewMatrices;
};
