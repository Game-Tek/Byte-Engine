#pragma once

#include "Core.h"
#include <GTSL/Time.h>

class PlayableAsset
{
public:

	void Update(const GTSL::Microseconds deltaTime) { elapsedTime += deltaTime; }

	[[nodiscard]] GTSL::Microseconds GetCurrentFrame() const { return elapsedTime; }
	[[nodiscard]] uint32 GetTotalFrameCount() const { return totalFrames; }

private:
	GTSL::Microseconds elapsedTime;
	const uint64 totalFrames = 0;

};
