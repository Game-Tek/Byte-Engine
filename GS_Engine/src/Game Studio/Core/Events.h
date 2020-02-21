#pragma once

#include <condition_variable>

class auto_event
{
    //https://vorbrodt.blog/2019/02/08/event-objects/
public:
    explicit auto_event(const bool signaled = false) noexcept : signaled(signaled) {}

    void signal() noexcept
    {
        std::unique_lock<std::mutex> lock(mutex);
        signaled = true;
        cv.notify_one();
    }

    void wait() noexcept
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
