#pragma once

#include <GTSL/Math/Vectors.h>
#include <GTSL/Math/Vectors.h>
#include <GTSL/Id.h>
#include <map>
#include <GTSL/HashMap.h>
#include <GTSL/Math/Matrix4.h>


#include "ByteEngine/Application/AllocatorReferences.h"

struct SkinnableVertex
{
	GTSL::Vector3 Position;
	GTSL::Vector3 Normal;
	uint16 BoneIDs[8];
	GTSL::Vector2 TextureCoordinates;
};

struct Joint
{
	GTSL::Id64 Name;
	uint16 Parent;
	GTSL::Matrix4 Offset;
};

class Skeleton
{
	GTSL::HashMap<Id, Joint, BE::PAR> bones;

public:
};
