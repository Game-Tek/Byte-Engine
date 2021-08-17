#pragma once

#include "AudioCore.h"

namespace AAL
{
	/**
	 * \brief Interface for an audio device. Creates and manages an audio device, endpoint and buffer.
	 */
	class AudioDevice
	{
	public:
		AudioDevice() = default;
		~AudioDevice() = default;

		enum class BufferSamplePlacement : GTSL::uint8 { BLOCKS, INTERLEAVED };
		
		struct MixFormat
		{
			GTSL::uint8 NumberOfChannels;
			GTSL::uint32 SamplesPerSecond;
			GTSL::uint8 BitsPerSample;
			
			GTSL::uint8 GetBytesPerSample() const { return BitsPerSample / 8; }
			
			/**
			 * \brief Frame size, in bytes. The frame size is the minimum atomic unit of data for the format.
			 * Frane size is equal to the product of NumberChannels and BitsPerSample divided by 8 (bytes per sample).
			 * Software must process a multiple of BlockAlignment bytes of data at a time. Data written to and read from a device must always start at the beginning of a block.
			 * For example, it is illegal to start playback of PCM data in the middle of a sample (that is, on a non-block-aligned boundary).
			 */
			GTSL::uint16 GetFrameSize() const { return static_cast<GTSL::uint16>(NumberOfChannels) * GetBytesPerSample(); }
		};
		
		struct CreateInfo
		{
		};

	};
}