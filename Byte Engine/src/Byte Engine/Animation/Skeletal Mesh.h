#pragma once

#include "Math/Vector3.h"
#include "Utility/TextureCoordinates.h"
#include <GTSL/Id.h>
#include "Math/Matrix4.h"
#include <map>

struct SkinnableVertex
{
	Vector3 Position;
	TextureCoordinates TextureCoordinates;
	Vector3 Normal;
	uint16 BoneIDs[4];
};

struct Joint
{
	GTSL::Id64 Name;
	uint16 Parent;
	Matrix4 Offset;
};

class Skeleton
{
	std::map<GTSL::Id64, Joint> bones;

public:
};
