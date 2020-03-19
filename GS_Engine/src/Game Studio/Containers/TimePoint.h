#pragma once

#include "Core.h"

/**
 * \brief Represent a time duration or point. Minimum unit of time it can express is 1 microsecond.
 */
class TimePoint
{
	//microseconds
	uint_64 time = 0;

	constexpr TimePoint(const uint_64 a) : time(a) {}
public:
	TimePoint() = default;

	template<typename T>
	static constexpr TimePoint CreateFromMicroSeconds(const T a);
	static constexpr TimePoint CreateFromMicroSeconds(const uint_64 a) { return a; }

	template<typename T>
	static constexpr TimePoint CreateFromSeconds(const T a);
	static constexpr TimePoint CreateFromSeconds(const uint_64 a) { return a * 1000000; }

	template<typename T>
	T Milliseconds() const;
	template<>
	[[nodiscard]] uint_64 Milliseconds() const { return time / 1000; }
	template<>
	[[nodiscard]] float Milliseconds() const { return time / 1000.0f; }
	template<>
	[[nodiscard]] double Milliseconds() const { return time / 1000.0; }

	template<typename T>
	T Seconds() const;
	template<>
	[[nodiscard]] uint_64 Seconds() const { return time / 1000000; }
	template<>
	[[nodiscard]] float Seconds() const { return time / 1000000.0f; }
	template<>
	[[nodiscard]] double Seconds() const { return time / 1000000.0; }

	template<typename T>
	T Minutes() const;
	template<>
	[[nodiscard]] uint_64 Minutes() const { return time / 10000000; }
	template<>
	[[nodiscard]] float Minutes() const { return time / 10000000.0f; }
	template<>
	[[nodiscard]] double Minutes() const { return time / 10000000.0; }

	TimePoint operator+(const TimePoint& other) const { return time + other.time; }
	TimePoint& operator+=(const uint_64 other) { time += other; return *this; }
	TimePoint operator-(const TimePoint& other) const { return time - other.time; }
	bool operator>(const TimePoint& other) const { return time > other.time; }
	bool operator<(const TimePoint& other) const { return time < other.time; }

	TimePoint& operator=(const TimePoint& other) { time = other.time; return *this; }
};