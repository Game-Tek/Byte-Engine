#pragma once
#include "RenderCore.h"

namespace GAL
{
	/**
	 * \brief Object to achieve host-device synchronization.
	 */
	class Synchronizer {
	public:
		enum class Type {
			FENCE, SEMAPHORE, EVENT
		};
	};
}
