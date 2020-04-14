#include "AudioResourceManager.h"
#include <fstream>
#include <GTSL/Id.h>
#include <GTSL/System.h>

bool AudioResourceManager::LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo)
{
	auto search_result = resources.find(GTSL::Id64(loadResourceInfo.ResourceName));

	if (search_result == resources.end())
	{
		GTSL::String path(255);
		GTSL::System::GetRunningPath(path);
		path += "resources/";
		path += loadResourceInfo.ResourceName;
		path +=".wav";
		
		std::ifstream Input(path.c_str(), std::ios::in | std::ios::binary); //Open file as binary
		
		if (Input.is_open()) //If file is valid
		{
			auto data = search_result->second;
			
			Input.seekg(0, std::ios::end); //Search for end
			uint64 FileLength = Input.tellg(); //Get file length
			Input.seekg(0, std::ios::beg); //Move file pointer back to beginning

			InStream in_archive(&Input);

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
				return false;
			}

			in_archive.Read(&overall_size);
			in_archive.Read(4, wave);
			in_archive.Read(4, fmt_chunk_marker);
			in_archive.Read(&length_of_fmt);
			in_archive.Read(&format_type);
			in_archive.Read(&channels);
			switch (channels)
			{
			case 1: data.AudioChannelCount = AudioChannelCount::CHANNELS_MONO; break;
			case 2: data.AudioChannelCount = AudioChannelCount::CHANNELS_STEREO; break;
			case 6: data.AudioChannelCount = AudioChannelCount::CHANNELS_5_1; break;
			case 8: data.AudioChannelCount = AudioChannelCount::CHANNELS_7_1; break;
			default: throw std::exception("Channel count not supported!");
			}

			in_archive.Read(&sample_rate);
			switch (sample_rate)
			{
			case 44100: data.AudioSampleRate = AudioSampleRate::KHZ_44_1; break;
			case 48000: data.AudioSampleRate = AudioSampleRate::KHZ_48; break;
			case 96000: data.AudioSampleRate = AudioSampleRate::KHZ_96; break;
			default: throw std::exception("Sample rate not supported!");
			}

			in_archive.Read(&byterate);
			in_archive.Read(&block_align);
			in_archive.Read(&bits_per_sample);
			switch (bits_per_sample)
			{
			case 8: data.AudioBitDepth = AudioBitDepth::BIT_DEPTH_8; break;
			case 16: data.AudioBitDepth = AudioBitDepth::BIT_DEPTH_16; break;
			case 24: data.AudioBitDepth = AudioBitDepth::BIT_DEPTH_24; break;
			default: throw std::exception("Bit-depth not supported!");
			}

			in_archive.Read(4, data_chunk_header);
			in_archive.Read(&data_size);

			data.Bytes = GTSL::FixedVector<byte>(data_size);

			in_archive.Read(data_size, data.Bytes.GetData());

			return true;
		}
		
		Input.close();
		return false;
	}

	onResourceLoadInfo.ResourceData = static_cast<ResourceData*>(&search_result->second);
	
	return true;
}

void AudioResourceManager::LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo)
{
}
