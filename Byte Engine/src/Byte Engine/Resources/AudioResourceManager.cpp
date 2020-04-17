#include "AudioResourceManager.h"

#include <fstream>
#include <GTSL/Id.h>
#include <GTSL/System.h>

AudioResourceManager::AudioResourceData* AudioResourceManager::TryGetResource(const GTSL::String& name)
{
	const GTSL::Id64 hashed_name(name);

	{
		resourceMapMutex.ReadLock();
		if (resources.contains(hashed_name))
		{
			resourceMapMutex.ReadUnlock();
			resourceMapMutex.WriteLock();
			auto& res = resources.at(hashed_name);
			res.IncrementReferences();
			resourceMapMutex.WriteUnlock();
			return &res;
		}
		resourceMapMutex.ReadUnlock();
	}

	GTSL::String path(255, &transientAllocator);
	GTSL::System::GetRunningPath(path);
	path += "resources/";
	path += name;
	path += '.';
	path += "wav";

	std::ifstream input(path.c_str(), std::ios::in | std::ios::binary);

	if (input.is_open())
	{
		AudioResourceData data;

		GTSL::InStream in_archive(&input);

		unsigned char riff[4];                      // RIFF string
		unsigned int overall_size;               // overall size of file in bytes
		unsigned char wave[4];                      // WAVE string
		unsigned char fmt_chunk_marker[4];          // fmt string with trailing null char
		unsigned int length_of_fmt;                 // length of the format data
		unsigned int format_type;                   // format type. 1-PCM, 3- IEEE float, 6 - 8bit A law, 7 - 8bit mu law
		unsigned int channels;                      // no.of channels
		unsigned int sample_rate;                   // sampling rate (blocks per second)
		unsigned int byterate;                      // SampleRate * NumChannels * BitsPerSample/8
		unsigned int block_align;                   // NumChannels * BitsPerSample/8
		unsigned int bits_per_sample;               // bits per sample, 8- 8bits, 16- 16 bits etc
		unsigned char data_chunk_header[4];        // DATA string or FLLR string
		unsigned int data_size;                     // NumSamples * NumChannels * BitsPerSample/8 - size of the next chunk that will be read

		in_archive.Read(4, riff);
		if (riff[0] != 'r' || riff[1] != 'i' || riff[2] != 'f' || riff[3] != 'f')
		{
			return nullptr;
		}

		in_archive.Read(&overall_size);
		in_archive.Read(4, wave);
		in_archive.Read(4, fmt_chunk_marker);
		in_archive.Read(&length_of_fmt);
		in_archive.Read(&format_type);
		in_archive.Read(&channels);
		switch (channels)
		{
		case 1: data.AudioChannelCount = AAL::AudioChannelCount::CHANNELS_MONO; break;
		case 2: data.AudioChannelCount = AAL::AudioChannelCount::CHANNELS_STEREO; break;
		case 6: data.AudioChannelCount = AAL::AudioChannelCount::CHANNELS_5_1; break;
		case 8: data.AudioChannelCount = AAL::AudioChannelCount::CHANNELS_7_1; break;
		default: return nullptr;
		}

		in_archive.Read(&sample_rate);
		switch (sample_rate)
		{
		case 44100: data.AudioSampleRate = AAL::AudioSampleRate::KHZ_44_1; break;
		case 48000: data.AudioSampleRate = AAL::AudioSampleRate::KHZ_48; break;
		case 96000: data.AudioSampleRate = AAL::AudioSampleRate::KHZ_96; break;
		default: return nullptr;
		}

		in_archive.Read(&byterate);
		in_archive.Read(&block_align);
		in_archive.Read(&bits_per_sample);
		switch (bits_per_sample)
		{
		case 8: data.AudioBitDepth = AAL::AudioBitDepth::BIT_DEPTH_8; break;
		case 16: data.AudioBitDepth = AAL::AudioBitDepth::BIT_DEPTH_16; break;
		case 24: data.AudioBitDepth = AAL::AudioBitDepth::BIT_DEPTH_24; break;
		default: return nullptr;
		}

		in_archive.Read(4, data_chunk_header);
		in_archive.Read(&data_size);

		data.Bytes = GTSL::FixedVector<byte>(data_size, &bigAllocator);

		in_archive.Read(data_size, data.Bytes.GetData());

		resourceMapMutex.WriteLock();
		resources.emplace(hashed_name, GTSL::MakeTransferReference(data)).first->second.IncrementReferences();
		resourceMapMutex.WriteUnlock();
		return nullptr;
	}

	input.close();
	return nullptr;
}
