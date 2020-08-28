#pragma once

#include "ByteEngine/Core.h"
#include "ByteEngine/Object.h"

#include "ByteEngine/Debug/Logger.h"

#include <GTSL/Array.hpp>
#include <GTSL/Atomic.hpp>
#include <GTSL/Algorithm.h>
#include <GTSL/Semaphore.h>
#include <GTSL/Delegate.hpp>
#include <GTSL/BlockingQueue.h>
#include <GTSL/Thread.h>
#include <GTSL/Tuple.h>

//https://github.com/mvorbrodt/blog

class ThreadPool : public Object
{
	using TaskDelegate = GTSL::Delegate<void(ThreadPool*, void*)>;
	using Tasks = GTSL::Tuple<TaskDelegate, void*, GTSL::Semaphore*>;
public:
	explicit ThreadPool() : Object("Thread Pool"), queues(threadCount)
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
				
				GTSL::Get<TUPLE_LAMBDA_DELEGATE_INDEX>(task)(pool, &task);
			}
		};

		for (uint8 i = 0; i < threadCount; ++i)
		{
			//Constructing threads with function and I parameter
			threads.EmplaceBack(GetPersistentAllocator(), i, GTSL::Delegate<void(ThreadPool*, uint8)>::Create(workers_loop), this, i);
			threads[i].SetPriority(GTSL::Thread::Priority::HIGH);
		}
	}

	~ThreadPool()
	{
		for (auto& queue : queues) { queue.Done(); }
		for (auto& thread : threads) { thread.Join(GetPersistentAllocator()); }
	}

	template<typename F, typename... ARGS>
	void EnqueueTask(const GTSL::Delegate<F>& task, GTSL::Semaphore* semaphore, ARGS&&... args)
	{
		TaskInfo<F, ARGS...>* taskInfoAlloc = GTSL::New<TaskInfo<F, ARGS...>>(GetPersistentAllocator(), task, GTSL::ForwardRef<ARGS>(args)...);

		auto work = [](ThreadPool* threadPool, void* voidTask) -> void
		{
			Tasks* task = static_cast<Tasks*>(voidTask);
			TaskInfo<F, ARGS...>* taskInfo = static_cast<TaskInfo<F, ARGS...>*>(GTSL::Get<TUPLE_LAMBDA_TASK_INFO_INDEX>(*task));
			
			GTSL::Call(taskInfo->Delegate, taskInfo->Arguments);
			static_cast<GTSL::Semaphore*>(GTSL::Get<TUPLE_SEMAPHORE_INDEX>(*task))->Post();

			GTSL::Delete<TaskInfo<F, ARGS...>>(taskInfo, threadPool->GetPersistentAllocator());
		};
		
		const auto currentIndex = ++index;

		for (auto n = 0; n < threadCount * K; ++n)
		{
			//Try to Push work into queues, if success return else when Done looping place into some queue.

			if (queues[(currentIndex + n) % threadCount].TryPush(Tasks(TaskDelegate::Create(work), GTSL::MoveRef((void*)taskInfoAlloc), GTSL::MoveRef(semaphore)))) { return; }
		}

		queues[currentIndex % threadCount].Push(Tasks(TaskDelegate::Create(work), GTSL::MoveRef((void*)taskInfoAlloc), GTSL::MoveRef(semaphore)));
	}

private:
	inline const static uint8 threadCount{ static_cast<uint8>(GTSL::Thread::ThreadCount() - 1) };
	GTSL::Atomic<uint32> index{ 0 };

	static constexpr uint8 TUPLE_LAMBDA_DELEGATE_INDEX = 0;
	static constexpr uint8 TUPLE_LAMBDA_TASK_INFO_INDEX = 1;
	static constexpr uint8 TUPLE_SEMAPHORE_INDEX = 2;
	
	GTSL::Array<GTSL::BlockingQueue<Tasks>, 32> queues;
	GTSL::Array<GTSL::Thread, 32> threads;

	template<typename T, typename... ARGS>
	struct TaskInfo
	{
		TaskInfo(const GTSL::Delegate<T>& delegate, GTSL::Tuple<ARGS...>&& args) : Delegate(delegate), Arguments(GTSL::MoveRef(args))
		{
		}

		TaskInfo(const GTSL::Delegate<T>& delegate, ARGS&&... args) : Delegate(delegate), Arguments(GTSL::ForwardRef<ARGS>(args)...)
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