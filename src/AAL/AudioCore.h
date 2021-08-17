#pragma once

#include <GTSL/Core.h>

#include "GTSL/Math/Math.hpp"

namespace AAL
{
	/**
	 * \brief Names all possible audio channel counts.
	 */
	enum class AudioChannelCount : GTSL::uint8
	{
		/**
		 * \brief Audio is mono, has only one channel.
		 */
		CHANNELS_MONO = 1,

		/**
		 * \brief Audio is stereo, has two channels. Typical for speakers or headphones.
		 */
		CHANNELS_STEREO = 2,

		/**
		 * \brief Audio is in 5.1, has five channels and one sub-woofer channel. Typical for home cinema setups.
		 */
		CHANNELS_5_1 = 5,

		/**
		 * \brief Audio is in 7.1, has seven channels and one sub-woofer channel. Typical for home cinema setups.
		 */
		CHANNELS_7_1 = 7
	};

	/**
	 * \brief Names all possible audio bit depths.
	 */
	enum class AudioBitDepth : GTSL::uint8
	{
		/**
		 * \brief Audio bit depth is 8 bit.
		 */
		BIT_DEPTH_8 = 8,

		/**
		 * \brief Audio bit depth is 16 bit.
		 */
		BIT_DEPTH_16 = 16,

		/**
		 * \brief Audio bit depth is 24 bit.
		 */
		BIT_DEPTH_24 = 24,

		BIT_DEPTH_32 = 32
	};

	/**
	 * \brief Names all possible audio sample rates (kHz).
	 */
	enum class AudioSampleRate : GTSL::uint8
	{
		/**
		 * \brief Audio sample rate is 44.100 Hz.
		 */
		KHZ_44_1 = 44,

		/**
		* \brief Audio sample rate is 48.000 Hz.
		 */
		KHZ_48 = 48,

		/**
		* \brief Audio sample rate is 96.000 Hz.
		*/
		KHZ_96 = 96
	};

	/**
	 * \brief Names all possible audio output device types.
	 */
	enum class AudioOutputDeviceType : GTSL::uint8
	{
		/**
		 * \brief Audio output device are speakers.
		 */
		SPEAKERS,

		/**
		 * \brief Audio output device are headphones.
		 */
		HEADPHONES
	};

	enum class StreamShareMode : GTSL::uint8
	{
		SHARED,
		EXCLUSIVE
	};

	inline float dBToVolume(const float db) { return GTSL::Math::Power(10.0f, 0.05f * db); }

	inline float VolumeTodB(const float volume) { return 20.0f * GTSL::Math::Log10(volume); }
}
