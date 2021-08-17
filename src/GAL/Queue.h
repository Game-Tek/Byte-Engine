#pragma once

#include "RenderCore.h"
#include "GTSL/Core.h"

namespace GAL {
	class CommandList;
	class Semaphore;

	class Queue {
	public:
		struct WorkUnit final {
			const CommandList* CommandBuffer = nullptr;
			Semaphore* SignalSemaphore = nullptr;
			Semaphore* WaitSemaphore = nullptr;
			GTSL::uint64 SignalValue = 0, WaitValue = 0;
			/**
			 * \brief Pipeline stages at which each corresponding semaphore wait will occur.
			 */
			PipelineStage WaitPipelineStage;
		};
	};
}
