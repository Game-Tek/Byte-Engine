#include "AudioResource.h"

#include <fstream>

bool AudioResource::loadResource(const LoadResourceData& loadResourceData)
{
	std::ifstream Input(loadResourceData.FullPath.c_str(), std::ios::in | std::ios::binary); //Open file as binary

	if (Input.is_open()) //If file is valid
	{
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
			throw std::exception("No riff found!");
		}
		in_archive.Read(&overall_size);
		in_archive.Read(4, wave);
		if (wave[0] != 'w' || wave[1] != 'a' || wave[2] != 'v' || wave[3] != 'e')
		{
			throw std::exception("No wave found!");
		}
		in_archive.Read(4, fmt_chunk_marker);
		if (fmt_chunk_marker[0] != 'f' || fmt_chunk_marker[1] != 'm' || fmt_chunk_marker[2] != 't' || fmt_chunk_marker[3] != '\0')
		{
			throw std::exception("No fmt found!");
		}
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

		data.Bytes = DArray<byte>(data_size);

		in_archive.Read(data_size, data.Bytes.getData());
	}
	else
	{
		Input.close();
		return false;
	}

	Input.close();

	return true;
}
