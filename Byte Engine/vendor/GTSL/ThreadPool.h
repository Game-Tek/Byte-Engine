#pragma once

#include "Thread.h"
#include "BlockingQueue.h"
#include <future>
#include "Array.hpp"
#include "Delegate.h"

namespace GTSL
{
	//https://github.com/mvorbrodt/blog
	
	class ThreadPool
	{
	public:
		explicit ThreadPool(const uint32 numberOfThreads = Thread::ThreadCount()) : queues(numberOfThreads), threadCount(numberOfThreads)
		{
			if (!numberOfThreads) { throw std::invalid_argument("Invalid thread count!"); }

			//lambda
			auto worker_pop_function = [this](const uint8 i)
			{
				while (true)
				{
					Proc f;
					for (auto n = 0; n < threadCount * K; ++n)
					{
						if (queues[(i + n) % threadCount].TryPop(f)) { break; }
					}
					
					if (!f && !queues[i].Pop(f)) { break; }
					f();
				}
			};

			for (uint8 i = 0; i < numberOfThreads; ++i)
			{
				//Constructing threads with function and I parameter
				threads.EmplaceBack(worker_pop_function, i);
			}
		}

		~ThreadPool()
		{
			for (auto& queue : queues) { queue.Done(); }
			for (auto& thread : threads) { thread.Join(); }
		}

		template<typename F, typename... ARGS>
		void EnqueueWork(F&& f, ARGS&&... args)
		{
			auto work = [function = GTSL::MakeForwardReference<F>(f), arguments = std::make_tuple(GTSL::MakeForwardReference<ARGS>(args)...)]()
			{
				std::apply(function, arguments);
			};
			
			const auto currentIndex = index++;

			for (auto n = 0; n < threadCount * K; ++n)
			{
				//Try to Push work into queues, if success return else when Done looping place into some queue.
				
				if (queues[(currentIndex + n) % threadCount].TryPush(work)) { return; }
			}

			queues[currentIndex % threadCount].Push(GTSL::MakeTransferReference(work));
		}

	private:
		using Proc = Delegate<void()>;
		Array<BlockingQueue<Proc>, 64, uint8> queues;

		Array<Thread, 64, uint8> threads;

		const uint8 threadCount{ 0 };
		std::atomic_uint index{ 0 };

		/**
		 * \brief Number of times to loop around the queues to find one that is free.
		 */
		inline static constexpr uint8 K{ 2 };
	};
}