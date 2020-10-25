#pragma once

#include <assimp/Importer.hpp>
#include <assimp/scene.h>
#include <GTSL/FlatHashMap.h>
#include <GTSL/Math/Matrix4.h>



#include "ByteEngine/Core.h"
#include "ByteEngine/Id.h"
#include "ByteEngine/Application/AllocatorReferences.h"

struct Bone
{
	GTSL::Matrix4 Offset;
	uint32 AffectedBone[4];
	float32 EffectIntensity[4];
};

struct Skeleton
{
	GTSL::FlatHashMap<Bone, BE::PAR> Bones;
};

class AnimationResourceManager
{
public:
	AnimationResourceManager()
	{
		aiScene* scene;
		auto* mesh = scene->mMeshes[0];

		Skeleton skeleton;
		
		for(uint32 b = 0; b < mesh->mNumBones; ++b)
		{
			auto& bone = skeleton.Bones.Emplace(Id(mesh->mBones[b]->mName.C_Str()));
			
			for(uint32 w = 0; w < mesh->mBones[b]->mNumWeights; ++w)
			{
				bone.AffectedBone[w] = mesh->mBones[b]->mWeights[w].mVertexId;
				bone.EffectIntensity[w] = mesh->mBones[b]->mWeights[w].mWeight;
			}			

			//bone.Offset = mesh->mBones[b]->mOffsetMatrix;
		}

		for(uint32 a = 0; a < scene->mNumAnimations; ++a)
		{
			auto& animation = scene->mAnimations[a];
		}
	}
};
