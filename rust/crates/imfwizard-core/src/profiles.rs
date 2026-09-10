pub use postkit::profiles::{EncodingProfile, Platform, all_profiles, profile_for};

/// Map a CLI preset name to a delivery platform.
pub fn platform_from_name(name: &str) -> Option<Platform> {
    match name.to_lowercase().replace(['_', ' '], "-").as_str() {
        "dci-2k" | "theatrical-2k" | "theatricaldci2k" => Some(Platform::TheatricalDci2k),
        "dci-4k" | "theatrical-4k" | "theatricaldci4k" => Some(Platform::TheatricalDci4k),
        "netflix" => Some(Platform::Netflix),
        "disney" => Some(Platform::Disney),
        "hbo" => Some(Platform::Hbo),
        "archival" | "archivalpreservation" | "preservation" => {
            Some(Platform::ArchivalPreservation)
        }
        "broadcast" => Some(Platform::Broadcast),
        _ => None,
    }
}

pub fn required_audio_channels(profile: &EncodingProfile) -> Option<u32> {
    match profile.audio_channels.to_lowercase().as_str() {
        "mono" => Some(1),
        "stereo" => Some(2),
        "5.1" => Some(6),
        "7.1" => Some(8),
        "7.1.4" => Some(12),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMMERSIVE_AUDIO_LAYOUT: &str = "IAB";

    #[test]
    fn every_preset_audio_layout_names_a_channel_count_or_an_immersive_bitstream() {
        for profile in all_profiles() {
            if profile.audio_channels == IMMERSIVE_AUDIO_LAYOUT {
                assert_eq!(required_audio_channels(&profile), None);
                continue;
            }
            assert!(
                required_audio_channels(&profile).is_some(),
                "{} carries the unreadable audio layout {:?}",
                profile.name,
                profile.audio_channels
            );
        }
        assert_eq!(
            required_audio_channels(&profile_for(Platform::Disney)),
            Some(6)
        );
        assert_eq!(
            required_audio_channels(&profile_for(Platform::Netflix)),
            Some(6)
        );
        assert_eq!(required_audio_channels(&profile_for(Platform::Hbo)), None);
    }

    #[test]
    fn platform_from_name_maps_known_presets() {
        assert!(matches!(
            platform_from_name("netflix"),
            Some(Platform::Netflix)
        ));
        assert!(matches!(
            platform_from_name("DCI-4K"),
            Some(Platform::TheatricalDci4k)
        ));
        assert!(matches!(platform_from_name("hbo"), Some(Platform::Hbo)));
        // no public delivery specification
        assert!(platform_from_name("amazon").is_none());
        assert!(platform_from_name("apple").is_none());
        assert!(platform_from_name("nope").is_none());
    }
}
