#pragma once

#include <GTSL/Vector.hpp>
#include <GTSL/Math/Math.hpp>

#include "ByteEngine/Game/System.hpp"
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

	void PlayAnimation(const SkeletonHandle skeletonHandle, Id animationName)
	{
		GTSL::Microseconds elapsedTime, deltaTime;
		uint32 animationFrames, animationFPS;
		GTSL::Microseconds frameLength(GTSL::Seconds(1)); frameLength /= GTSL::Microseconds(60);
		
		GTSL::Vector3 time[64];
		
		elapsedTime += deltaTime;

		uint64 currentFrame = (elapsedTime / frameLength).GetCount();

		if(currentFrame > animationFrames) {
			elapsedTime -= GTSL::Microseconds(animationFrames) * frameLength;
			currentFrame = (elapsedTime / frameLength).GetCount();
		}
		
		uint64 nextFrame = currentFrame % animationFrames;
		float32 progressBetweenFrames = 0;

		auto newPos = GTSL::Math::Lerp(time[currentFrame], time[nextFrame], progressBetweenFrames);
	}

private:
	GTSL::StaticVector<Id, 8> skeletonsNames;
	GTSL::StaticVector<void, 8> skeletons;
	
	GTSL::StaticVector<void, 8> animations;
};
