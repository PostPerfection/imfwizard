pub use postkit::trailer::{
    RatingSystem, TrailerBand, TrailerOptions, TrailerResult, package_trailer,
};

const RATING_SYSTEMS: &[(&str, RatingSystem)] = &[
    ("mpaa", RatingSystem::Mpaa),
    ("bbfc", RatingSystem::Bbfc),
    ("fsk", RatingSystem::Fsk),
    ("custom", RatingSystem::Custom),
];

const BANDS: &[(&str, TrailerBand)] = &[
    ("green", TrailerBand::Green),
    ("red", TrailerBand::Red),
    ("yellow", TrailerBand::Yellow),
];

fn names<T: Copy>(table: &[(&str, T)]) -> String {
    table
        .iter()
        .map(|(name, _)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn rating_system_names() -> String {
    names(RATING_SYSTEMS)
}

pub fn band_names() -> String {
    names(BANDS)
}

/// Parse a rating system selector, erring with the ones that work.
pub fn rating_system_from_name(name: &str) -> Result<RatingSystem, String> {
    let wanted = name.to_lowercase();
    RATING_SYSTEMS
        .iter()
        .find(|(known, _)| *known == wanted)
        .map(|(_, system)| *system)
        .ok_or_else(|| {
            format!(
                "unknown rating system '{name}' (known: {})",
                rating_system_names()
            )
        })
}

/// Parse a band colour selector, erring with the ones that work.
pub fn band_from_name(name: &str) -> Result<TrailerBand, String> {
    let wanted = name.to_lowercase();
    BANDS
        .iter()
        .find(|(known, _)| *known == wanted)
        .map(|(_, band)| *band)
        .ok_or_else(|| format!("unknown band '{name}' (known: {})", band_names()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_named_rating_system_and_band_parses() {
        assert_eq!(
            rating_system_from_name("BBFC").unwrap(),
            RatingSystem::Bbfc,
            "the selector is case insensitive"
        );
        for (name, _) in RATING_SYSTEMS {
            rating_system_from_name(name).unwrap();
        }
        for (name, _) in BANDS {
            band_from_name(name).unwrap();
        }
    }

    #[test]
    fn an_unknown_selector_names_the_ones_that_work() {
        let error = rating_system_from_name("bbfg").unwrap_err();
        assert!(error.contains("bbfg") && error.contains("bbfc"), "{error}");
        let error = band_from_name("blue").unwrap_err();
        assert!(error.contains("blue") && error.contains("green"), "{error}");
    }
}
