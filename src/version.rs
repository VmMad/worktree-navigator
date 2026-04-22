pub const CURRENT_VERSION: &str = env!("WT_VERSION");

pub fn current_version() -> &'static str {
    CURRENT_VERSION
}

pub fn is_dev_build(version: &str) -> bool {
    version.trim().ends_with("-dev")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_dev_builds() {
        assert!(is_dev_build("1.2.3-dev"));
        assert!(is_dev_build(" 1.2.3-dev "));
        assert!(!is_dev_build("1.2.3"));
        assert!(!is_dev_build("1.2.3-dev.1"));
    }
}
