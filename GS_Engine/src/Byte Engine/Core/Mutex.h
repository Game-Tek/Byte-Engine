#pragma once

#include "Core.h"
#include "Signal.h"
#include <mutex>
#include <atomic>

class FastMutex
{
    //https://vorbrodt.blog/2019/02/12/fast-mutex/
public:
    FastMutex() : state(0) {}

    void Lock()
    {
        if (state.exchange(1, std::memory_order_acquire))
            while (state.exchange(2, std::memory_order_acquire))
                waitset.Wait();
    }

    void Unlock()
    {
        if (state.exchange(0, std::memory_order_release) == 2)
            waitset.Flag();
    }

private:
    std::atomic<uint32> state;
    Signal waitset;
};

class Mutex
{
    std::mutex mutex;

public:

    INLINE void Lock() { mutex.lock(); }
    INLINE bool TryLock() { return mutex.try_lock(); }
    INLINE void Unlock() { mutex.unlock(); }
};

template<class T>
class Lock
{
protected:
public:
};

template<>
class Lock<FastMutex>
{
    FastMutex* object = nullptr;

public:
    INLINE Lock(FastMutex& mutex)
    {
        object = &mutex;
        mutex.Lock();
    }

    INLINE ~Lock()
    {
        object->Unlock();
    }
};

template<>
class Lock<Mutex>
{
    Mutex* object = nullptr;

public:
    INLINE Lock(Mutex& mutex)
    {
        object = &mutex;
        mutex.Lock();
    }

    INLINE ~Lock()
    {
        object->Unlock();
    }
};