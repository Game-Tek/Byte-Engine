#include "Camera.h"
#include "Math/GSM.hpp"

void Camera::SetFocusDistance(const Vector3 & Object)
{
	FocusDistance = GSM::VectorLength(Transform.Position - Object);
	
	return;
}