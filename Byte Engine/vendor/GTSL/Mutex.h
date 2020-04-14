#pragma once

#include "Core.h"
#include "Signal.h"
#include <mutex>
#include <atomic>
#include <shared_mutex>

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
    Mutex() noexcept = default;
    ~Mutex() noexcept = default;
    Mutex(const Mutex & other) noexcept = delete;
    Mutex(Mutex && other) noexcept = delete;
    Mutex& operator=(const Mutex & other) = delete;
    Mutex& operator=(Mutex && other) = delete;
	
    void Lock() { mutex.lock(); }
    bool TryLock() { return mutex.try_lock(); }
    void Unlock() { mutex.unlock(); }
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
    explicit Lock(FastMutex& mutex) noexcept : object(&mutex) { mutex.Lock(); }
    ~Lock() noexcept { object->Unlock(); }
};

template<>
class Lock<Mutex>
{
    Mutex* object = nullptr;

public:
    explicit Lock(Mutex& mutex) noexcept : object(&mutex) { mutex.Lock(); }
    ~Lock() noexcept { object->Unlock(); }
};

class ReadWriteMutex
{
    std::shared_mutex sharedMutex;

public:
    ReadWriteMutex() noexcept = default;
    ~ReadWriteMutex() noexcept = default;
    ReadWriteMutex(const ReadWriteMutex& other) noexcept = delete;
    ReadWriteMutex(ReadWriteMutex&& other) noexcept = delete;
    ReadWriteMutex& operator=(const ReadWriteMutex& other) = delete;
    ReadWriteMutex& operator=(ReadWriteMutex&& other) = delete;
	
    void WriteLock() noexcept { sharedMutex.lock(); }
    void ReadLock() noexcept { sharedMutex.lock_shared(); }
    void WriteUnlock() noexcept { sharedMutex.unlock(); }
    void ReadUnlock() noexcept { sharedMutex.unlock_shared(); }
};

template<class T>
class ReadLock
{
};

template<class T>
class WriteLock
{
};

template<>
class ReadLock<ReadWriteMutex>
{
    ReadWriteMutex* readWriteMutex{ nullptr };
public:
    ReadLock(ReadWriteMutex& readWriteMutex) noexcept : readWriteMutex(&readWriteMutex) { readWriteMutex.ReadLock(); }
    ~ReadLock() noexcept { readWriteMutex->ReadUnlock(); }
};

template<>
class WriteLock<ReadWriteMutex>
{
    ReadWriteMutex* readWriteMutex{ nullptr };
public:
    WriteLock(ReadWriteMutex& readWriteMutex) noexcept : readWriteMutex(&readWriteMutex) { readWriteMutex.WriteLock(); }
    ~WriteLock() noexcept { readWriteMutex->WriteUnlock(); }
};