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
		explicit ThreadPool() : queues(threadCount)
		{	
			//lambda
			auto workers_loop = [this](const uint8 i)
			{
				while (true)
				{
					Delegate<void()> work;
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
				threads.EmplaceBack(workers_loop, i);
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

		template<typename F, typename... ARGS>
		void EnqueueWork(const Delegate<F>& delegate, ARGS&&... args)
		{
			auto work = [delegate, arguments = GTSL::MakeForwardReference<ARGS>(args)...]()
			{
				delegate(GTSL::MakeForwardReference<ARGS>(arguments)...);
			};
			
			const auto currentIndex = index++;

			for (auto n = 0; n < threadCount * K; ++n)
			{
				//Try to Push work into queues, if success return else when Done looping place into some queue.
				
				if (queues[(currentIndex + n) % threadCount].TryPush(work)) { return; }
			}

			queues[currentIndex % threadCount].Push(GTSL::MakeTransferReference(work));
		}

		template<typename F, typename... ARGS>
		[[nodiscard]] auto EnqueueTask(F&& f, ARGS&&... args) -> std::future<std::invoke_result_t<F, ARGS...>>
		{
			using task_return_type = std::invoke_result_t<F, ARGS...>;
			using task_type = std::packaged_task<task_return_type()>;

			auto task = std::make_shared<task_type>(std::bind(GTSL::MakeForwardReference<F>(f), GTSL::MakeForwardReference<ARGS>(args)...));
			auto work = [task]()
			{
				(*task)();
			};
			auto result = task->get_future();
			auto i = index++;

			for (auto n = 0; n < threadCount * K; ++n)
			{
				if (queues[(i + n) % threadCount].TryPush(work))
				{
					return result;
				}
			}

			queues[i % threadCount].Push(GTSL::MakeTransferReference(work));	

			return result;
		}
		
	private:
		Array<BlockingQueue<Delegate<void()>>, 64, uint8> queues;

		Array<Thread, 64, uint8> threads;

		inline const static uint8 threadCount{ Thread::ThreadCount() };
		std::atomic_uint index{ 0 };

		/**
		 * \brief Number of times to loop around the queues to find one that is free.
		 */
		inline static constexpr uint8 K{ 2 };
	};
}