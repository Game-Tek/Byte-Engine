#pragma once
#include "Core.h"
#include <type_traits>
#include <atomic>
#include "Semaphore.h"
#include <queue>

template<typename T>
class blocking_queue
{
public:
	template<typename Q = T>
	typename std::enable_if<std::is_copy_constructible<Q>::value, void>::
	type push(const T& item)
	{
		{
			std::unique_lock lock(m_mutex);
			m_queue.push(item);
		}
		m_ready.notify_one();
	}

	template<typename Q = T>
	typename std::enable_if<std::is_move_constructible<Q>::value, void>::
	type push(T&& item)
	{
		{
			std::unique_lock lock(m_mutex);
			m_queue.emplace(std::forward<T>(item));
		}
		m_ready.notify_one();
	}

	template<typename Q = T>
	typename std::enable_if<std::is_copy_constructible<Q>::value, bool>::
	type try_push(const T& item)
	{
		{
			std::unique_lock lock(m_mutex, std::try_to_lock);
			if (!lock)
				return false;
			m_queue.push(item);
		}
		m_ready.notify_one();
		return true;
	}

	template<typename Q = T>
	typename std::enable_if<std::is_move_constructible<Q>::value, bool>::
	type try_push(T&& item)
	{
		{
			std::unique_lock lock(m_mutex, std::try_to_lock);
			if (!lock)
				return false;
			m_queue.emplace(std::forward<T>(item));
		}
		m_ready.notify_one();
		return true;
	}

	template<typename Q = T>
	typename std::enable_if<std::is_copy_assignable<Q>::value && !std::is_move_assignable<Q>::value, bool>::
	type pop(T& item)
	{
		std::unique_lock lock(m_mutex);
		while (m_queue.empty() && !m_done)
			m_ready.wait(lock);
		if (m_queue.empty())
			return false;
		item = m_queue.front();
		m_queue.pop();
		return true;
	}

	template<typename Q = T>
	typename std::enable_if<std::is_move_assignable<Q>::value, bool>::type
		pop(T& item)
	{
		std::unique_lock lock(m_mutex);
		while (m_queue.empty() && !m_done)
			m_ready.wait(lock);
		if (m_queue.empty())
			return false;
		item = std::move(m_queue.front());
		m_queue.pop();
		return true;
	}

	template<typename Q = T>
	typename std::enable_if<std::is_copy_assignable<Q>::value && !std::is_move_assignable<Q>::value, bool>::
	type try_pop(T& item)
	{
		std::unique_lock lock(m_mutex, std::try_to_lock);
		if (!lock || m_queue.empty())
			return false;
		item = m_queue.front();
		m_queue.pop();
		return true;
	}

	template<typename Q = T>
	typename std::enable_if<std::is_move_assignable<Q>::value, bool>::
	type try_pop(T& item)
	{
		std::unique_lock lock(m_mutex, std::try_to_lock);
		if (!lock || m_queue.empty())
			return false;
		item = std::move(m_queue.front());
		m_queue.pop();
		return true;
	}

	void done() noexcept
	{
		{
			std::unique_lock lock(m_mutex);
			m_done = true;
		}
		m_ready.notify_all();
	}

	[[nodiscard]] bool empty() const noexcept
	{
		std::scoped_lock lock(m_mutex);
		return m_queue.empty();
	}

	[[nodiscard]] uint32 size() const noexcept
	{
		std::scoped_lock lock(m_mutex);
		return m_queue.size();
	}

private:
	std::queue<T> m_queue;
	mutable std::mutex m_mutex;
	std::condition_variable m_ready;
	bool m_done = false;
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
			throw std::invalid_argument("Invalid queue size!");
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
	typename std::enable_if<!std::is_move_assignable<Q>::value && std::is_nothrow_copy_assignable<Q>::value, void>::
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
		std::is_move_assignable<Q>::value &&
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
		!std::is_move_assignable<Q>::value &&
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
	typename std::enable_if<std::is_move_assignable<Q>::value && std::is_nothrow_move_assignable<Q>::value, bool>::
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
