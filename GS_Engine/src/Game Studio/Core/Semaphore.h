#pragma once
#include "Core.h"

#include <atomic>
#include <condition_variable>

class semaphore
{
public:
	explicit semaphore(const int32 count) noexcept : m_count(count)
	{
        GS_ASSERT(count > -1, "Count must be more than -1.")
    }

    void post() noexcept
    {
        {
            std::unique_lock<std::mutex> lock(m_mutex);
            ++m_count;
        }
        m_cv.notify_one();
    }

    void wait() noexcept
    {
        std::unique_lock<std::mutex> lock(m_mutex);
        m_cv.wait(lock, [&]() { return m_count != 0; });
        --m_count;
    }

private:
    int32 m_count = 0;
    std::mutex m_mutex;
    std::condition_variable m_cv;
};

class fast_semaphore
{
public:
	explicit fast_semaphore(const int32 count) noexcept : m_count(count), m_semaphore(0) {}

    void post()
    {
        std::atomic_thread_fence(std::memory_order_release);
        const auto count = m_count.fetch_add(1, std::memory_order_relaxed);
        if (count < 0)
            m_semaphore.post();
    }

    void wait()
    {
	    const auto count = m_count.fetch_sub(1, std::memory_order_relaxed);
        if (count < 1)
            m_semaphore.wait();
        std::atomic_thread_fence(std::memory_order_acquire);
    }

private:
    std::atomic<int32> m_count;
    semaphore m_semaphore;
};