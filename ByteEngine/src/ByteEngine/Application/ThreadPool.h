#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Array.hpp>
#include <GTSL/Atomic.hpp>
#include <GTSL/Delegate.hpp>
#include <GTSL/BlockingQueue.h>
#include <GTSL/ConditionVariable.h>
#include <thread>
#include <GTSL/Thread.h>

//https://github.com/mvorbrodt/blog

class ThreadPool
{
public:
	explicit ThreadPool() : queues(threadCount)
	{
		//lambda
		auto workers_loop = [this](const uint8 i)
		{
			while (true)
			{
				GTSL::Delegate<void()> work;
				for (auto n = 0; n < threadCount * K; ++n)
				{
					if (queues[(i + n) % threadCount].TryPop(work)) { break; }
				}

				if (!work && !queues[i].Pop(work)) { break; }
				work();
			}
		};

		for (uint8 i = 0; i < threadCount; ++i)
		{
			//Constructing threads with function and I parameter
			//threads.EmplaceBack(workers_loop, i);
			threads.EmplaceBack(GTSL::Delegate<void(const uint8)>::Create(workers_loop), i);
			//threads.SetThreadPriority(HIGH);
		}
	}

	~ThreadPool()
	{
		for (auto& queue : queues) { queue.Done(); }
		for (auto& thread : threads) { thread.Join(); }
	}

	template<typename F, typename... ARGS>
	void EnqueueTask(const GTSL::Delegate<F>& delegate, GTSL::Semaphore* conditionVariable, ARGS&&... args)
	{	
		auto work = [delegate, conditionVariable, ... args = GTSL::MakeForwardReference<ARGS>(args)]()
		{
			delegate(GTSL::MakeForwardReference<ARGS>(args)...);
			conditionVariable->Post();
		};

		const auto current_index = ++index;

		for (auto n = 0; n < threadCount * K; ++n)
		{
			//Try to Push work into queues, if success return else when Done looping place into some queue.

			if (queues[(current_index + n) % threadCount].TryPush(GTSL::Delegate<void()>::Create(work))) { return; }
		}

		queues[current_index % threadCount].Push(GTSL::Delegate<void()>::Create(work));
	}

private:
	inline const static uint8 threadCount{ static_cast<uint8>(std::thread::hardware_concurrency() - 1) };
	GTSL::Atomic<uint32> index{ 0 };
	
	GTSL::Array<GTSL::BlockingQueue<GTSL::Delegate<void()>>, 64> queues;
	GTSL::Array<GTSL::Thread, 64> threads;


	/**
	 * \brief Number of times to loop around the queues to find one that is free.
	 */
	inline static constexpr uint8 K{ 2 };
};