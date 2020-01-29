#pragma once

#include "Core.h"

/**
 * \brief Names all possible audio channel counts.
 */
enum class AudioChannelCount : uint8
{
	/**
	 * \brief Audio is mono, has only one channel.
	 */
	CHANNELS_MONO,

	/**
	 * \brief Audio is stereo, has two channels. Typical for speakers or headphones.
	 */
	CHANNELS_STEREO,

	/**
	 * \brief Audio is in 5.1, has five channels and one sub-woofer channel. Typical for home cinema setups.
	 */
	CHANNELS_5_1,

	/**
	 * \brief Audio is in 7.1, has seven channels and one sub-woofer channel. Typical for home cinema setups.
	 */
	CHANNELS_7_1
};

/**
 * \brief Names all possible audio bit depths.
 */
enum class AudioBitDepth : uint8
{
	/**
	 * \brief Audio bit depth is 8 bit.
	 */
	BIT_DEPTH_8,

	/**
	 * \brief Audio bit depth is 16 bit.
	 */
	BIT_DEPTH_16,

	/**
	 * \brief Audio bit depth is 24 bit.
	 */
	BIT_DEPTH_24
};

/**
 * \brief Names all possible audio sample rates (kHz).
 */
enum class AudioSampleRate : uint8
{
	/**
	 * \brief Audio sample rate is 44.100 Hz.
	 */
	KHZ_44_1,

	/**
	* \brief Audio sample rate is 48.000 Hz.
	 */
	KHZ_48,

	/**
	* \brief Audio sample rate is 96.000 Hz.
	*/
	KHZ_96
};

/**
 * \brief Names all possible audio output device types.
 */
enum class AudioOutputDeviceType : uint8
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
