pub const PACKAGE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const GIT_COMMIT: &str = env!("PAX_GIT_COMMIT");
pub const GIT_DATE: &str = env!("PAX_GIT_DATE");
pub const VERSION_STRING: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("PAX_GIT_COMMIT"),
    ", ",
    env!("PAX_GIT_DATE"),
    ")"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_string_includes_build_metadata() {
        assert!(VERSION_STRING.contains(PACKAGE_VERSION));
        assert!(VERSION_STRING.contains(GIT_COMMIT));
        assert!(VERSION_STRING.contains(GIT_DATE));
    }
}
