#pragma once

#include "Core.h"

namespace GTSL
{
	/**
	 * \brief Represent a time duration or point. Minimum unit of time it can express is 1 microsecond.
	 */
	class TimePoint
	{
		//microseconds
		uint64 time = 0;

		constexpr TimePoint(const uint64 a) noexcept : time(a) {}
	public:
		TimePoint() = default;

		static constexpr TimePoint CreateFromMicroseconds(const uint64 a) noexcept { return a; }

		static constexpr TimePoint CreateFromMilliseconds(const uint64 a) noexcept { return a * 1000; }

		static constexpr TimePoint CreateFromSeconds(const uint64 a) noexcept { return a * 1000000; }

		template<typename T>
		T Microseconds() const;
		template<>
		[[nodiscard]] uint64 Microseconds() const { return time; }
		template<>
		[[nodiscard]] float  Microseconds() const { return time; }
		template<>
		[[nodiscard]] double Microseconds() const { return time; }

		template<typename T>
		T Milliseconds() const;
		template<>
		[[nodiscard]] uint64 Milliseconds() const { return time / 1000; }
		template<>
		[[nodiscard]] float  Milliseconds() const { return time / 1000.0f; }
		template<>
		[[nodiscard]] double Milliseconds() const { return time / 1000.0; }

		template<typename T>
		T Seconds() const;
		template<>
		[[nodiscard]] uint64 Seconds() const { return time / 1000000; }
		template<>
		[[nodiscard]] float  Seconds() const { return time / 1000000.0f; }
		template<>
		[[nodiscard]] double Seconds() const { return time / 1000000.0; }

		TimePoint operator+(const TimePoint& other) const { return time + other.time; }
		TimePoint& operator+=(const TimePoint& other) { time += other.time; return *this; }
		TimePoint operator-(const TimePoint& other) const { return time - other.time; }
		TimePoint& operator-=(const TimePoint& other) { time -= other.time; return *this; }
		TimePoint operator*(const float other) const { return time * static_cast<double>(other); }
		bool operator>(const TimePoint& other) const { return time > other.time; }
		bool operator<(const TimePoint& other) const { return time < other.time; }

		TimePoint& operator=(const TimePoint& other) { time = other.time; return *this; }
	};
}