#pragma once
#include "Core.h"
#include <type_traits>
#include <atomic>
#include "Semaphore.h"
#include <queue>

namespace GTSL
{
	template<typename T>
	class BlockingQueue
	{
	public:
		template<typename Q = T>
		typename std::enable_if<std::is_copy_constructible<Q>::value, void>::
		type Push(const T& item)
		{
			{
				std::unique_lock lock(mutex);
				queue.push(item);
			}
			ready.notify_one();
		}

		template<typename Q = T>
		typename std::enable_if<std::is_move_constructible<Q>::value, void>::
			type Push(T&& item)
		{
			{
				std::unique_lock lock(mutex);
				queue.emplace(GTSL::MakeForwardReference<T>(item));
			}
			ready.notify_one();
		}

		template<typename Q = T>
		typename std::enable_if<std::is_copy_constructible<Q>::value, bool>::
			type TryPush(const T& item)
		{
			{
				std::unique_lock lock(mutex, std::try_to_lock);
				if (!lock) { return false; }
				queue.push(item);
			}
			ready.notify_one();
			return true;
		}

		template<typename Q = T>
		typename std::enable_if<std::is_move_constructible<Q>::value, bool>::
			type TryPush(T&& item)
		{
			{
				std::unique_lock lock(mutex, std::try_to_lock);
				if (!lock)
					return false;
				queue.emplace(GTSL::MakeForwardReference<T>(item));
			}
			ready.notify_one();
			return true;
		}

		template<typename Q = T>
		typename std::enable_if<std::is_copy_assignable<Q>::value && !std::is_move_assignable<Q>::value, bool>::
			type Pop(T& item)
		{
			std::unique_lock lock(mutex);
			while (queue.empty() && !done) { ready.wait(lock); }
			if (queue.empty()) { return false; }
			item = queue.front();
			queue.pop();
			return true;
		}

		template<typename Q = T>
		typename std::enable_if<std::is_move_assignable<Q>::value, bool>::
		type
		Pop(T& item)
		{
			std::unique_lock lock(mutex);
			while (queue.empty() && !done) { ready.wait(lock); }
			if (queue.empty()) { return false; }
			item = GTSL::MakeTransferReference(queue.front());
			queue.pop();
			return true;
		}

		template<typename Q = T>
		typename std::enable_if<std::is_copy_assignable<Q>::value && !std::is_move_assignable<Q>::value, bool>::
			type TryPop(T& item)
		{
			std::unique_lock lock(mutex, std::try_to_lock);
			if (!lock || queue.empty())
				return false;
			item = queue.front();
			queue.pop();
			return true;
		}

		template<typename Q = T>
		typename std::enable_if<typename std::is_move_assignable<Q>::value, bool>::
		type TryPop(T& item)
		{
			std::unique_lock lock(mutex, std::try_to_lock);
			if (!lock || queue.empty())
				return false;
			item = std::move(queue.front());
			queue.pop();
			return true;
		}

		void Done() noexcept
		{
			{
				std::unique_lock lock(mutex);
				done = true;
			}
			ready.notify_all();
		}

		[[nodiscard]] bool IsEmpty() const noexcept
		{
			std::scoped_lock lock(mutex);
			return queue.empty();
		}

		[[nodiscard]] uint32 GetSize() const noexcept
		{
			std::scoped_lock lock(mutex);
			return queue.size();
		}

	private:
		std::queue<T> queue;
		mutable std::mutex mutex;
		std::condition_variable ready;
		bool done = false;
	};

	template<typename T>
	class atomic_blocking_queue
	{
		//https://github.com/mvorbrodt/blog/blob/master/src/queue.hpp
	public:
		explicit atomic_blocking_queue(const uint32 size)
			: m_size(size), m_pushIndex(0), m_popIndex(0), m_count(0),
			m_data((T*)operator new(size * sizeof(T))),
			m_openSlots(size), m_fullSlots(0)
		{
			if (!size)
				throw std::invalid_argument("Invalid queue GetSize!");
		}

		~atomic_blocking_queue() noexcept
		{
			while (m_count--)
			{
				m_data[m_popIndex].~T();
				m_popIndex = ++m_popIndex % m_size;
			}
			operator delete(m_data);
		}

		template<typename Q = T>
		typename std::enable_if<std::is_nothrow_copy_constructible<Q>::value, void>::
			type push(const T& item) noexcept
		{
			m_openSlots.Wait();

			auto pushIndex = m_pushIndex.fetch_add(1);
			new (m_data + (pushIndex % m_size)) T(item);
			++m_count;

			auto expected = m_pushIndex.load();
			while (!m_pushIndex.compare_exchange_weak(expected, m_pushIndex % m_size))
				expected = m_pushIndex.load();

			m_fullSlots.Post();
		}

		template<typename Q = T>
		typename std::enable_if<std::is_nothrow_move_constructible<Q>::value, void>::
			type push(T&& item) noexcept
		{
			m_openSlots.Wait();

			auto pushIndex = m_pushIndex.fetch_add(1);
			new (m_data + (pushIndex % m_size)) T(std::move(item));
			++m_count;

			auto expected = m_pushIndex.load();
			while (!m_pushIndex.compare_exchange_weak(expected, m_pushIndex % m_size))
				expected = m_pushIndex.load();

			m_fullSlots.Post();
		}

		template<typename Q = T>
		typename std::enable_if<std::is_nothrow_copy_constructible<Q>::value, bool>::
			type try_push(const T& item) noexcept
		{
			auto result = m_openSlots.wait_for(std::chrono::seconds(0));
			if (!result)
				return false;

			auto pushIndex = m_pushIndex.fetch_add(1);
			new (m_data + (pushIndex % m_size)) T(item);
			++m_count;

			auto expected = m_pushIndex.load();
			while (!m_pushIndex.compare_exchange_weak(expected, m_pushIndex % m_size))
				expected = m_pushIndex.load();

			m_fullSlots.Post();
			return true;
		}

		template<typename Q = T>
		typename std::enable_if<std::is_nothrow_move_constructible<Q>::value, bool>::
			type try_push(T&& item) noexcept
		{
			auto result = m_openSlots.wait_for(std::chrono::seconds(0));
			if (!result)
				return false;

			auto pushIndex = m_pushIndex.fetch_add(1);
			new (m_data + (pushIndex % m_size)) T(std::move(item));
			++m_count;

			auto expected = m_pushIndex.load();
			while (!m_pushIndex.compare_exchange_weak(expected, m_pushIndex % m_size))
				expected = m_pushIndex.load();

			m_fullSlots.Post();
			return true;
		}

		template<typename Q = T>
		typename std::enable_if<!std::is_move_assignable<Q>::value&& std::is_nothrow_copy_assignable<Q>::value, void>::
			type pop(T& item) noexcept
		{
			m_fullSlots.Wait();

			auto popIndex = m_popIndex.fetch_add(1);
			item = m_data[popIndex % m_size];
			m_data[popIndex % m_size].~T();
			--m_count;

			auto expected = m_popIndex.load();
			while (!m_popIndex.compare_exchange_weak(expected, m_popIndex % m_size))
				expected = m_popIndex.load();

			m_openSlots.Post();
		}

		template<typename Q = T>
		typename std::enable_if<
			std::is_move_assignable<Q>::value&&
			std::is_nothrow_move_assignable<Q>::value, void>::type
			pop(T& item) noexcept
		{
			m_fullSlots.Wait();

			auto popIndex = m_popIndex.fetch_add(1);
			item = std::move(m_data[popIndex % m_size]);
			m_data[popIndex % m_size].~T();
			--m_count;

			auto expected = m_popIndex.load();
			while (!m_popIndex.compare_exchange_weak(expected, m_popIndex % m_size))
				expected = m_popIndex.load();

			m_openSlots.Post();
		}

		template<typename Q = T>
		typename std::enable_if<
			!std::is_move_assignable<Q>::value&&
			std::is_nothrow_copy_assignable<Q>::value, bool>::type
			try_pop(T& item) noexcept
		{
			auto result = m_fullSlots.wait_for(std::chrono::seconds(0));
			if (!result)
				return false;

			auto popIndex = m_popIndex.fetch_add(1);
			item = m_data[popIndex % m_size];
			m_data[popIndex % m_size].~T();
			--m_count;

			auto expected = m_popIndex.load();
			while (!m_popIndex.compare_exchange_weak(expected, m_popIndex % m_size))
				expected = m_popIndex.load();

			m_openSlots.Post();
			return true;
		}

		template<typename Q = T>
		typename std::enable_if<std::is_move_assignable<Q>::value&& std::is_nothrow_move_assignable<Q>::value, bool>::
			type try_pop(T& item) noexcept
		{
			auto result = m_fullSlots.wait_for(std::chrono::seconds(0));
			if (!result)
				return false;

			auto popIndex = m_popIndex.fetch_add(1);
			item = std::move(m_data[popIndex % m_size]);
			m_data[popIndex % m_size].~T();
			--m_count;

			auto expected = m_popIndex.load();
			while (!m_popIndex.compare_exchange_weak(expected, m_popIndex % m_size))
				expected = m_popIndex.load();

			m_openSlots.Post();
			return true;
		}

		[[nodiscard]] bool empty() const noexcept { return m_count == 0; }

		[[nodiscard]] bool full() const noexcept { return m_count == m_size; }

		[[nodiscard]] uint32 size() const noexcept { return m_count; }

		[[nodiscard]] uint32 capacity() const noexcept { return m_size; }

	private:
		const uint32 m_size = 0;
		std::atomic_uint m_pushIndex;
		std::atomic_uint m_popIndex;
		std::atomic_uint m_count;
		T* m_data = nullptr;

		Semaphore m_openSlots;
		Semaphore m_fullSlots;
	};
}