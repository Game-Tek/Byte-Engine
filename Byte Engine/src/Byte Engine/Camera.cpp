#include "Camera.h"
#include "Math/BEM.hpp"

void Camera::SetFocusDistance(const Vector3& Object)
{
	focusDistance = BEM::Length(Transform.Position - Object);

	return;
}
