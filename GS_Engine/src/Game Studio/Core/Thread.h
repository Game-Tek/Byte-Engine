#pragma once

#include "Core.h"

#include <thread>
#include <mutex>

class Thread
{
	std::thread thread;

public:
	template<typename F, typename... P>
	explicit Thread(F& lambda_, P... params_) : thread(lambda_, &params_...) {}
	
	INLINE void Join() { thread.join(); }
	INLINE void Detach() { thread.detach(); }
	
	[[nodiscard]] INLINE bool CanBeJoined() const { return thread.joinable(); }
};

class Mutex
{
	std::mutex mutex;

public:

	INLINE void Lock() { mutex.lock(); }
	INLINE bool TryLock() { return mutex.try_lock(); }
	INLINE void Unlock() { mutex.unlock(); }
	
};
