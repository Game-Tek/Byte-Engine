#pragma once

using byte = unsigned char;
using uint8 = unsigned char;
using int8 = char;
using uint16 = unsigned short;
using int16 = short;
using uint32 = unsigned int;
using int32 = int;
using uint64 = unsigned long long;
using int64 = long long;

namespace GTSL
{
	template <typename T>
	struct RemoveReference { using type = T; using const_type_ref = const T; };

	template <typename T>
	struct RemoveReference<T&> { using type = T; using const_type_ref = const T&; };
	
	template <typename T>
	struct RemoveReference<T&&> { using type = T; using const_type_ref = const T&&; };
	
	template<typename T>
	[[nodiscard]] constexpr T&& MakeForwardReference(typename RemoveReference<T>::type& args) noexcept { return static_cast<T&&>(args); }

	template <typename T>
	[[nodiscard]] constexpr typename RemoveReference<T>::type&& MakeTransferReference(T&& arg) noexcept { return static_cast<typename RemoveReference<T>::type&&>(arg); }
}
