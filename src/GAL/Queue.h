#pragma once

#include "RenderCore.h"
#include "GTSL/Core.h"

namespace GAL {
	class CommandList;
	class Semaphore;

	class Queue {
	public:
		struct WorkUnit final {
			struct SemaphoreOperationInfo {
				Semaphore* Semaphore = nullptr;
				PipelineStage PipelineStage;
			};

			GTSL::Range<SemaphoreOperationInfo*> SignalSemaphores, WaitSemaphores;
			GTSL::Range<const CommandList**> CommandLists;
		};
	};
}
