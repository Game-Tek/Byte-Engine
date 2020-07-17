#include "AudioResourceManager.h"

#include <GTSL/Buffer.h>
#include <GTSL/Filesystem.h>
#include <GTSL/Serialize.h>

#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Debug/Assert.h"
#include <AAL/AudioCore.h>

AudioResourceManager::AudioResourceManager() : ResourceManager("AudioResourceManager")
{
	GTSL::StaticString<512> query_path, package_path, resources_path, index_path;
	query_path += BE::Application::Get()->GetPathToApplication(); query_path += "/resources/"; query_path += "*.wav";
	resources_path += BE::Application::Get()->GetPathToApplication(); resources_path += "/resources/";
	index_path += BE::Application::Get()->GetPathToApplication(); index_path += "/resources/Audio.beidx";
	package_path += BE::Application::Get()->GetPathToApplication(); package_path += "/resources/Audio.bepkg";

	indexFile.OpenFile(index_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
	packageFile.OpenFile(package_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::LEAVE_CONTENTS);
	
	GTSL::Buffer file_buffer; file_buffer.Allocate(2048 * 2048, 32, GetTransientAllocator());

	if(indexFile.ReadFile(file_buffer))
	{
		GTSL::Extract(audioResourceInfos, file_buffer, GetPersistentAllocator());
	}
	else
	{
		::new(&audioResourceInfos) GTSL::FlatHashMap<AudioResourceInfo>(8, 0.25, GetPersistentAllocator());
	}
	
	auto load = [&](const GTSL::FileQuery::QueryResult& queryResult)
	{
		auto file_path = resources_path;
		file_path += queryResult.FileNameWithExtension;
		auto name = queryResult.FileNameWithExtension; name.Drop(name.FindLast('.'));
		const auto hashed_name = GTSL::Id64(name.operator GTSL::Ranger<const char>());

		if (!audioResourceInfos.Find(hashed_name))
		{
			GTSL::File query_file;
			query_file.OpenFile(file_path, static_cast<uint8>(GTSL::File::AccessMode::READ), GTSL::File::OpenMode::LEAVE_CONTENTS);

			query_file.ReadFile(file_buffer);

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

			Extract(overall_size, file_buffer, GetTransientAllocator());
			file_buffer.ReadBytes(4, wave);
			file_buffer.ReadBytes(4, fmt_chunk_marker);
			Extract(length_of_fmt, file_buffer, GetTransientAllocator());
			Extract(format_type, file_buffer, GetTransientAllocator());
			Extract(channels, file_buffer, GetTransientAllocator());
			switch (channels)
			{
			case 1: data.AudioChannelCount = (uint8)AAL::AudioChannelCount::CHANNELS_MONO; break;
			case 2: data.AudioChannelCount = (uint8)AAL::AudioChannelCount::CHANNELS_STEREO; break;
			case 6: data.AudioChannelCount = (uint8)AAL::AudioChannelCount::CHANNELS_5_1; break;
			case 8: data.AudioChannelCount = (uint8)AAL::AudioChannelCount::CHANNELS_7_1; break;
			default: break;
			}

			Extract(sample_rate, file_buffer, GetTransientAllocator());
			switch (sample_rate)
			{
			case 44100: data.AudioSampleRate = (uint8)AAL::AudioSampleRate::KHZ_44_1; break;
			case 48000: data.AudioSampleRate = (uint8)AAL::AudioSampleRate::KHZ_48; break;
			case 96000: data.AudioSampleRate = (uint8)AAL::AudioSampleRate::KHZ_96; break;
			default:break;
			}

			Extract(byte_rate, file_buffer, GetTransientAllocator());
			Extract(block_align, file_buffer, GetTransientAllocator());
			Extract(bits_per_sample, file_buffer, GetTransientAllocator());
			switch (bits_per_sample)
			{
			case 8: data.AudioBitDepth =  (uint8)AAL::AudioBitDepth::BIT_DEPTH_8; break;
			case 16: data.AudioBitDepth = (uint8)AAL::AudioBitDepth::BIT_DEPTH_16; break;
			case 24: data.AudioBitDepth = (uint8)AAL::AudioBitDepth::BIT_DEPTH_24; break;
			default:break;
			}

			file_buffer.ReadBytes(4, data_chunk_header);
			Extract(data_size, file_buffer, GetTransientAllocator());

			data.ByteOffset = (uint32)packageFile.GetFileSize();

			packageFile.WriteToFile(GTSL::Ranger<byte>(data_size, file_buffer.GetData() + file_buffer.GetReadPosition()));

			audioResourceInfos.Emplace(GetPersistentAllocator(), hashed_name, data);

			query_file.CloseFile();
		}
	};
	
	GTSL::FileQuery file_query(query_path);
	GTSL::ForEach(file_query, load);

	indexFile.CloseFile();
	indexFile.OpenFile(index_path, (uint8)GTSL::File::AccessMode::WRITE | (uint8)GTSL::File::AccessMode::READ, GTSL::File::OpenMode::CLEAR);

	file_buffer.Resize(0);
	GTSL::Insert(audioResourceInfos, file_buffer, GetTransientAllocator());
	indexFile.WriteToFile(file_buffer);
	
	file_buffer.Free(32, GetTransientAllocator());
}

AudioResourceManager::~AudioResourceManager()
{
	packageFile.CloseFile(); indexFile.CloseFile();
	audioResourceInfos.Free(GetPersistentAllocator());
}

void Insert(const AudioResourceManager::AudioResourceInfo& audioResourceInfo, GTSL::Buffer& buffer,	const GTSL::AllocatorReference& allocatorReference)
{
	GTSL::Insert(audioResourceInfo.AudioBitDepth, buffer, allocatorReference);
	GTSL::Insert(audioResourceInfo.AudioChannelCount, buffer, allocatorReference);
	GTSL::Insert(audioResourceInfo.AudioSampleRate, buffer, allocatorReference);
	GTSL::Insert(audioResourceInfo.ByteOffset, buffer, allocatorReference);
}

void Extract(AudioResourceManager::AudioResourceInfo& audioResourceInfo, GTSL::Buffer& buffer, const GTSL::AllocatorReference& allocatorReference)
{
	GTSL::Extract(audioResourceInfo.AudioBitDepth, buffer, allocatorReference);
	GTSL::Extract(audioResourceInfo.AudioChannelCount, buffer, allocatorReference);
	GTSL::Extract(audioResourceInfo.AudioSampleRate, buffer, allocatorReference);
	GTSL::Extract(audioResourceInfo.ByteOffset, buffer, allocatorReference);
}

void AudioResourceManager::LoadAudioAsset(const LoadAudioAssetInfo& loadAudioAssetInfo)
{
	auto& audio_resource_info = audioResourceInfos.At(loadAudioAssetInfo.Name);
	
	if(!audioAssets.Find(loadAudioAssetInfo.Name))
	{
		indexFile.SetPointer(audio_resource_info.ByteOffset, GTSL::File::MoveFrom::BEGIN);
		//packageFile.ReadFromFile()
	}

	//handle resource is loaded
}
