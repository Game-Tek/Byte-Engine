#pragma once

#include <GTSL/Array.hpp>

#include "ByteEngine/Game/System.h"
#include "ByteEngine/Handle.hpp"

MAKE_HANDLE(uint32, Skeleton)

class AnimationSystem : public System
{
public:
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	SkeletonHandle CreateSkeleton()
	{
		auto skeletonIndex = skeletons.EmplaceBack();

		return SkeletonHandle(skeletonIndex);
	}

	void PlayAnimation(const SkeletonHandle skeletonHandle, Id animationName);

private:
	GTSL::Array<Id, 8> skeletonsNames;
	GTSL::Array<void, 8> skeletons;
	
	GTSL::Array<void, 8> animations;
};
