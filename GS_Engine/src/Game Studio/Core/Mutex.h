#pragma once

#include "Core.h"
#include "Events.h"
#include <mutex>
#include <atomic>

class fast_mutex
{
    //https://vorbrodt.blog/2019/02/12/fast-mutex/
public:
    fast_mutex() : state(0) {}

    void lock()
    {
        if (state.exchange(1, std::memory_order_acquire))
            while (state.exchange(2, std::memory_order_acquire))
                waitset.wait();
    }

    void unlock()
    {
        if (state.exchange(0, std::memory_order_release) == 2)
            waitset.signal();
    }

private:
    std::atomic<uint32> state;
    auto_event waitset;
};

class Mutex
{
    std::mutex mutex;

public:

    INLINE void Lock() { mutex.lock(); }
    INLINE bool TryLock() { return mutex.try_lock(); }
    INLINE void Unlock() { mutex.unlock(); }
};