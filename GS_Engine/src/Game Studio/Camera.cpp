#include "Camera.h"
#include "Math/GSM.hpp"

void Camera::SetFocusDistance(const Vector3& Object)
{
	FocusDistance = GSM::Length(Transform.Position - Object);

	return;
}
