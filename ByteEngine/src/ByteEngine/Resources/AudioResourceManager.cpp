#include "AudioResourceManager.h"

#include <GTSL/Buffer.hpp>
#include <GTSL/Filesystem.h>
#include <GTSL/Serialize.h>

#include "ByteEngine/Debug/Assert.h"
#include <AAL/AudioCore.h>

#include "ByteEngine/Application/Application.h"

AudioResourceManager::AudioResourceManager() : ResourceManager("AudioResourceManager"), audioResourceInfos(8, 0.25, GetPersistentAllocator())
{
	GTSL::StaticString<512> query_path, package_path, resources_path, index_path;
	query_path += BE::Application::Get()->GetPathToApplication(); query_path += "/resources/"; query_path += "*.wav";
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/";
	index_path += BE::Application::Get()->GetPathToApplication(); index_path += "/resources/Audio.beidx";
	package_path += BE::Application::Get()->GetPathToApplication(); package_path += "/resources/Audio.bepkg";

	indexFile.OpenFile(index_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
	packageFile.OpenFile(package_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
	
	GTSL::Buffer<BE::TAR> file_buffer; file_buffer.Allocate(2048 * 2048, 32, GetTransientAllocator());
	
	if(indexFile.ReadFile(file_buffer.GetBufferInterface()))
	{
		GTSL::Extract(audioResourceInfos, file_buffer);
	}
	
	auto load = [&](const GTSL::FileQuery::QueryResult& queryResult)
	{
		auto file_path = resources_path;
		file_path += queryResult.FileNameWithExtension;
		auto name = queryResult.FileNameWithExtension; name.Drop(name.FindLast('.'));
		const auto hashed_name = GTSL::Id64(name);

		if (!audioResourceInfos.Find(hashed_name))
		{
			GTSL::File query_file;
			query_file.OpenFile(file_path, static_cast<uint8>(GTSL::File::AccessMode::READ), GTSL::File::OpenMode::LEAVE_CONTENTS);

			query_file.ReadFile(file_buffer.GetBufferInterface());

			AudioResourceInfo data;

			uint8 riff[4];                      // RIFF string
			uint32 overall_size = 0;               // overall size of file in bytes
			uint8 wave[4];                      // WAVE string
			uint8 fmt_chunk_marker[4];          // fmt string with trailing null char
			uint32 length_of_fmt = 0;                 // length of the format data
			uint16 format_type = 0;                   // format type. 1-PCM, 3- IEEE float, 6 - 8bit A law, 7 - 8bit mu law
			uint16 channels = 0;                      // no.of channels
			uint32 sample_rate = 0;                   // sampling rate (blocks per second)
			uint32 byte_rate = 0;                      // SampleRate * NumChannels * BitsPerSample/8
			uint16 block_align = 0;                   // NumChannels * BitsPerSample/8
			uint16 bits_per_sample = 0;               // bits per sample, 8- 8bits, 16- 16 bits etc
			uint8 data_chunk_header[4];        // DATA string or FLLR string
			uint32 data_size = 0;                     // NumSamples * NumChannels * BitsPerSample/8 - size of the next chunk that will be read

			file_buffer.ReadBytes(4, riff);
			BE_ASSERT(riff[0] != 'r' || riff[1] != 'i' || riff[2] != 'f' || riff[3] != 'f', "No RIFF");

			Extract(overall_size, file_buffer);
			file_buffer.ReadBytes(4, wave);
			file_buffer.ReadBytes(4, fmt_chunk_marker);
			Extract(length_of_fmt, file_buffer);
			Extract(format_type, file_buffer);
			Extract(channels, file_buffer);
			switch (channels)
			{
			case 1: data.AudioChannelCount = (uint8)AAL::AudioChannelCount::CHANNELS_MONO; break;
			case 2: data.AudioChannelCount = (uint8)AAL::AudioChannelCount::CHANNELS_STEREO; break;
			case 6: data.AudioChannelCount = (uint8)AAL::AudioChannelCount::CHANNELS_5_1; break;
			case 8: data.AudioChannelCount = (uint8)AAL::AudioChannelCount::CHANNELS_7_1; break;
			default: break;
			}

			Extract(sample_rate, file_buffer);
			switch (sample_rate)
			{
			case 44100: data.AudioSampleRate = (uint8)AAL::AudioSampleRate::KHZ_44_1; break;
			case 48000: data.AudioSampleRate = (uint8)AAL::AudioSampleRate::KHZ_48; break;
			case 96000: data.AudioSampleRate = (uint8)AAL::AudioSampleRate::KHZ_96; break;
			default:break;
			}

			Extract(byte_rate, file_buffer);
			Extract(block_align, file_buffer);
			Extract(bits_per_sample, file_buffer);
			switch (bits_per_sample)
			{
			case 8: data.AudioBitDepth =  (uint8)AAL::AudioBitDepth::BIT_DEPTH_8; break;
			case 16: data.AudioBitDepth = (uint8)AAL::AudioBitDepth::BIT_DEPTH_16; break;
			case 24: data.AudioBitDepth = (uint8)AAL::AudioBitDepth::BIT_DEPTH_24; break;
			default:break;
			}

			
			file_buffer.ReadBytes(4, data_chunk_header);
			Extract(data_size, file_buffer);
			
			data.Frames = data_size / channels / (bits_per_sample / 8);

			data.ByteOffset = (uint32)packageFile.GetFileSize();

			packageFile.WriteToFile(GTSL::Range<const byte*>(data_size, file_buffer.GetData() + file_buffer.GetReadPosition()));

			audioResourceInfos.Emplace(hashed_name, data);
		}
	};
	
	GTSL::FileQuery file_query(query_path);
	GTSL::ForEach(file_query, load);

	file_buffer.Resize(0);
	GTSL::Insert(audioResourceInfos, file_buffer);
	indexFile.WriteToFile(file_buffer);
}

AudioResourceManager::~AudioResourceManager()
{
}

void AudioResourceManager::LoadAudioAsset(const LoadAudioAssetInfo& loadAudioAssetInfo)
{
	auto& audio_resource_info = audioResourceInfos.At(loadAudioAssetInfo.Name);
	
	if(!audioBytes.Find(loadAudioAssetInfo.Name))
	{
		packageFile.SetPointer(audio_resource_info.ByteOffset, GTSL::File::MoveFrom::BEGIN);
		auto& bytes = audioBytes.At(loadAudioAssetInfo.Name);
		auto allocSize = audio_resource_info.AudioChannelCount * (audio_resource_info.AudioBitDepth / 8) * audio_resource_info.Frames;
		bytes.Allocate(allocSize, 8, GetPersistentAllocator());
		packageFile.ReadFromFile(GTSL::Range<byte*>(allocSize, bytes.GetData()));
		bytes.Resize(allocSize);
	}

	//handle resource is loaded
}
