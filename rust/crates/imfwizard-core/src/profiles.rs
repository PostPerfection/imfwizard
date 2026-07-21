pub use postkit::profiles::{EncodingProfile, Platform, all_profiles, profile_for};

/// Map a CLI preset name to a delivery platform.
pub fn platform_from_name(name: &str) -> Option<Platform> {
    match name.to_lowercase().replace(['_', ' '], "-").as_str() {
        "dci-2k" | "theatrical-2k" | "theatricaldci2k" => Some(Platform::TheatricalDci2k),
        "dci-4k" | "theatrical-4k" | "theatricaldci4k" => Some(Platform::TheatricalDci4k),
        "netflix" => Some(Platform::Netflix),
        "amazon" | "amazon-prime" | "prime" => Some(Platform::AmazonPrime),
        "disney" => Some(Platform::Disney),
        "apple" => Some(Platform::Apple),
        "hbo" => Some(Platform::Hbo),
        "archival" | "archivalpreservation" | "preservation" => {
            Some(Platform::ArchivalPreservation)
        }
        "broadcast" => Some(Platform::Broadcast),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_from_name_maps_known_presets() {
        assert!(matches!(platform_from_name("netflix"), Some(Platform::Netflix)));
        assert!(matches!(platform_from_name("DCI-4K"), Some(Platform::TheatricalDci4k)));
        assert!(matches!(platform_from_name("amazon_prime"), Some(Platform::AmazonPrime)));
        assert!(platform_from_name("nope").is_none());
    }
}
