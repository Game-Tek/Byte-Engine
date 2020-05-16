#pragma once

#include "Core.h"

class PlayableAsset
{
	uint32 totalFrames = 0;
	float currentFrame = 0;
	uint8 framesPerSecond = 0;

public:

	void Update(const float deltaTime);

	[[nodiscard]] float GetCurrentFrame() const { return currentFrame; }
	[[nodiscard]] uint32 GetTotalFrameCount() const { return totalFrames; }
	[[nodiscard]] uint8 GetFramesPerSecond() const { return framesPerSecond; }
};
