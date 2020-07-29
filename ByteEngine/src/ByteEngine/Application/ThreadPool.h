#pragma once

#include "ByteEngine/Core.h"

#include <GTSL/Array.hpp>
#include <GTSL/Atomic.hpp>
#include <GTSL/Algorithm.h>
#include <GTSL/Semaphore.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/BlockingQueue.h>
#include <GTSL/Thread.h>
#include <GTSL/Tuple.h>

//https://github.com/mvorbrodt/blog

class ThreadPool
{
public:
	explicit ThreadPool() : queues(threadCount)
	{
		//lambda
		auto workers_loop = [](ThreadPool* pool, const uint8 i)
		{
			while (true)
			{		
				Tasks task;
				
				for (auto n = 0; n < threadCount * K; ++n)
				{
					if (pool->queues[(i + n) % threadCount].TryPop(task)) { break; }
				}

				if (!GTSL::Get<TUPLE_LAMBDA_DELEGATE_INDEX>(task) && !pool->queues[i].Pop(task)) { break;	}
				
				GTSL::Get<TUPLE_LAMBDA_DELEGATE_INDEX>(task)(&task);
			}
		};

		for (uint8 i = 0; i < threadCount; ++i)
		{
			//Constructing threads with function and I parameter
			threads.EmplaceBack(GTSL::Delegate<void(ThreadPool*, uint8)>::Create(workers_loop), this, i);
			threads[i].SetPriority(GTSL::Thread::Priority::HIGH);
		}
	}

	~ThreadPool()
	{
		for (auto& queue : queues) { queue.Done(); }
		for (auto& thread : threads) { thread.Join(); }
	}

	template<typename F, typename P, typename... ARGS, typename... PARGS>
	void EnqueueTask(const GTSL::Delegate<F>& task, const GTSL::Delegate<P>& post_task, GTSL::Semaphore* semaphore, GTSL::Tuple<PARGS...>&& post_args, ARGS&&... args)
	{
		TaskInfo<F, ARGS...>* task_info = new TaskInfo<F, ARGS...>(task, GTSL::MakeForwardReference<ARGS>(args)...);
		TaskInfo<P, PARGS...>* post_task_info = new TaskInfo<P, PARGS...>(post_task, GTSL::MakeTransferReference(post_args));

		auto work = [](void* voidTask) -> void
		{
			Tasks* task = static_cast<Tasks*>(voidTask);
			TaskInfo<F, ARGS...>* task_info = static_cast<TaskInfo<F, ARGS...>*>(GTSL::Get<TUPLE_LAMBDA_TASK_INFO_INDEX>(*task));
			TaskInfo<P, PARGS...>* post_task_info = static_cast<TaskInfo<P, PARGS...>*>(GTSL::Get<TUPLE_LAMBDA_POST_TASK_INDEX>(*task));
			
			GTSL::Call(task_info->Delegate, task_info->Arguments);
			static_cast<GTSL::Semaphore*>(GTSL::Get<TUPLE_SEMAPHORE_INDEX>(*task))->Post();
			GTSL::Call(post_task_info->Delegate, post_task_info->Arguments);

			delete task_info;
			delete post_task_info;
		};
		
		const auto current_index = ++index;

		for (auto n = 0; n < threadCount * K; ++n)
		{
			//Try to Push work into queues, if success return else when Done looping place into some queue.

			if (queues[(current_index + n) % threadCount].TryPush(Tasks(GTSL::Delegate<void(void*)>::Create(work), GTSL::MakeTransferReference((void*)task_info), GTSL::MakeTransferReference((void*)post_task_info), GTSL::MakeTransferReference(semaphore)))) { return; }
		}

		queues[current_index % threadCount].Push(Tasks(GTSL::Delegate<void(void*)>::Create(work), GTSL::MakeTransferReference((void*)task_info), GTSL::MakeTransferReference((void*)post_task_info), GTSL::MakeTransferReference(semaphore)));
	}

private:
	inline const static uint8 threadCount{ static_cast<uint8>(GTSL::Thread::ThreadCount() - 1) };
	GTSL::Atomic<uint32> index{ 0 };

	static constexpr uint8 TUPLE_LAMBDA_DELEGATE_INDEX = 0;
	static constexpr uint8 TUPLE_LAMBDA_TASK_INFO_INDEX = 1;
	static constexpr uint8 TUPLE_LAMBDA_POST_TASK_INDEX = 2;
	static constexpr uint8 TUPLE_SEMAPHORE_INDEX = 3;
	
	using Tasks = GTSL::Tuple<GTSL::Delegate<void(void*)>, void*, void*, GTSL::Semaphore*>;
	
	GTSL::Array<GTSL::BlockingQueue<Tasks>, 64> queues;
	GTSL::Array<GTSL::Thread, 64> threads;

	template<typename T, typename... ARGS>
	struct TaskInfo
	{
		TaskInfo(const GTSL::Delegate<T>& delegate, GTSL::Tuple<ARGS...>&& args) : Delegate(delegate), Arguments(GTSL::MakeTransferReference(args))
		{
		}

		TaskInfo(const GTSL::Delegate<T>& delegate, ARGS&&... args) : Delegate(delegate), Arguments(GTSL::MakeForwardReference<ARGS>(args)...)
		{
		}
		
		GTSL::Delegate<T> Delegate;
		GTSL::Tuple<ARGS...> Arguments;
	};
	/**
	 * \brief Number of times to loop around the queues to find one that is free.
	 */
	inline static constexpr uint8 K{ 2 };
};