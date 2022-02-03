#pragma once

template<std::size_t N>
struct fixed_string {
	constexpr fixed_string(const char(&foo)[N + 1]) {
		std::copy_n(foo, N + 1, data);
	}

	auto operator<=>(const fixed_string&) const = default;
	char data[N + 1] = {};
};

template<std::size_t N>
fixed_string(const char(&str)[N])->fixed_string<N - 1>;

template<fixed_string TAG, typename T>
struct tag_and_value {
	T value;
};

template<fixed_string TAG>
struct arg_type {
	template<typename T>
	constexpr auto operator=(T t) const {
		return tag_and_value<TAG, T>{ std::move(t) };
	}
};

template<fixed_string TAG>
inline constexpr auto arg = arg_type<TAG>{};

template<fixed_string TAG, typename T>
struct member {
	//template<typename OTHER>
	//constexpr member(tag_and_value<TAG, OTHER> tv) : value(std::move(tv.value)) {}
};