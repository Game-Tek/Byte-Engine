#include "Camera.h"
#include <GTM/GTM.hpp>

void Camera::SetFocusDistance(const Vector3& Object)
{
	focusDistance = GTM::Length(Transform.Position - Object);

	return;
}
