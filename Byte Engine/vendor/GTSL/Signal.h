#pragma once

#include <condition_variable>

class Signal
{
    //https://vorbrodt.blog/2019/02/08/event-objects/
public:
    explicit Signal(const bool signaled = false) noexcept : signaled(signaled) {}

    void Flag() noexcept
    {
        std::unique_lock<std::mutex> lock(mutex);
        signaled = true;
        cv.notify_one();
    }

    void Wait() noexcept
    {
        std::unique_lock<std::mutex> lock(mutex);
        cv.wait(lock, [&]() { return signaled != false; });
        signaled = false;
    }

private:
    bool signaled = false;
    std::mutex mutex;
    std::condition_variable cv;
};
