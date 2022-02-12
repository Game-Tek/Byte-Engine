#pragma once

#include "RenderCore.h"
#include "GTSL/Core.h"

namespace GAL {
	class CommandList;
	class Semaphore;

	class Queue {
	public:
		template<class S>
		struct WorkUnit final {
			struct SynchronizerOperationInfo {
				S* Synchronizer = nullptr;
				PipelineStage PipelineStage;
			};

			GTSL::Range<SynchronizerOperationInfo*> Signal, Wait;
			GTSL::Range<const CommandList**> CommandLists;
		};
	};
}
