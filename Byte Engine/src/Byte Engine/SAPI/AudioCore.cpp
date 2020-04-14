#include "AudioCore.h"

#include <GTM/GTM.hpp>

float dBToVolume(const float db) { return GTM::Power(10.0f, 0.05f * db); }

float VolumeTodB(const float volume) { return 20.0f * GTM::Log10(volume); }
