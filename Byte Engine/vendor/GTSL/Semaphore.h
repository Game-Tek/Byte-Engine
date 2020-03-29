#pragma once
#include "Core.h"

#include <atomic>
#include <condition_variable>

class Semaphore
{
public:
	explicit Semaphore(const int32 count) noexcept : count(count)
	{
        GTSL_ASSERT(count > -1, "Count must be more than -1.")
    }

    void Post() noexcept
    {
        {
            std::unique_lock<std::mutex> lock(mutex);
            ++count;
        }
        cv.notify_one();
    }

    void Wait() noexcept
    {
        std::unique_lock<std::mutex> lock(mutex);
        cv.wait(lock, [&]() { return count != 0; });
        --count;
    }

private:
    int32 count = 0;
    std::mutex mutex;
    std::condition_variable cv;
};

class FastSemaphore
{
public:
	explicit FastSemaphore(const int32 count) noexcept : count(count), semaphore(0) {}

    void Post()
    {
        std::atomic_thread_fence(std::memory_order_release);
        const auto new_count = count.fetch_add(1, std::memory_order_relaxed);
        if (new_count < 0)
            semaphore.Post();
    }

    void Wait()
    {
	    const auto new_count = count.fetch_sub(1, std::memory_order_relaxed);
        if (new_count < 1)
            semaphore.Wait();
        std::atomic_thread_fence(std::memory_order_acquire);
    }

private:
    std::atomic<int32> count;
    Semaphore semaphore;
};