#pragma once

#include "Containers/Id.h"
#include "Containers/HashMap.hpp"

#include "Player.h"
#include "Containers/Pair.h"
#include "Containers/Array.hpp"

class AudioBuffer;

class Effect
{
	/**
	* \brief Defines the effect's name. Used to refer to it.
	*/
	Id effectName;

	/**
	 * \brief Determines the effects intensity when used in a channel.
	 */
	float effectIntensity = 0.0f;
	
public:
	virtual ~Effect();
	
	virtual void Process(const AudioBuffer& _AudioBuffer) = 0;
};

struct EffectRemoveParameters
{
	/**
	 * \brief Determines the time it takes for this effect to be killed.\n
	 * If KillTime is 0 the effect will be deleted immediately.
	 */
	float KillTime = 0.0f;

	/**
	 * \brief Pointer to the function to be used for fading out the effect. If any fading out is applied at all.
	 */
	void(*FadeFunction)() = nullptr;
};

class Mixer
{
	class Channel
	{
		friend class Mixer;
		
		/**
		 * \brief Defines the type for a Pair holding a bool to determine whether the sound is virtualized, and a Player* to know which Player to grab the data from.
		 */
		using PlayingSounds = Pair<bool, Player*>;


		/**
		 * \brief Determines how strong this channel sounds.
		 */
		float mixVolume = 0.0f;

		/**
		 * \brief Defines the channel's name. Used to refer to it from the mixer.
		 */
		Id channelName;

		/**
		 * \brief Holds an array of sounds which are to be played.
		 */
		FVector<PlayingSounds> playingSounds;
		
		/**
		 * \brief Holds the collection of effects this channel has.
		 */
		Array<Effect*, 10> effects;

	public:
		~Channel()
		{
			for(auto& e : effects)
			{
				delete e;
			}
		}
		
		void SetMixVolume(const float _MixVolume) { mixVolume = _MixVolume; }

		template<class _T>
		Effect* AddEffect(const Effect& _NewEffect)
		{
			Effect* new_effect = new _T();
			effects.push_back(new_effect);
			return new_effect;
		}

		void RemoveEffect(const EffectRemoveParameters& _ERP);
	};
	
	/**
	 * \brief Stores every channel available.
	 */
	HashMap<Channel> channels;
	
public:
	void OnUpdate();
	
	void RegisterNewChannel(const Channel& _Channel)
	{
		//channels.TryEmplace;
		channels.Get(0).effects;
	}

	Channel& GetChannel(const Id& _Id)
	{
		return channels.Get(_Id.GetID());
	}
};
