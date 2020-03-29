#pragma once

#include "Core.h"

#include <thread>
#include <mutex>
#include <atomic>

class Thread
{
	std::thread thread;

public:
	template <typename F, typename... P>
	explicit Thread(F& lambda_, P ... params_) : thread(lambda_, &params_...)
	{
	}

	INLINE void Join() { thread.join(); }
	INLINE void Detach() { thread.detach(); }

	[[nodiscard]] INLINE bool CanBeJoined() const { return thread.joinable(); }
};