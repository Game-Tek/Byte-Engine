#include "Camera.h"

#include <GTSL/Math/Math.hpp>

void Camera::SetFocusDistance(const GTSL::Vector3& Object)
{
	focusDistance = GTSL::Math::Length(Transform.Position - Object);

	return;
}
