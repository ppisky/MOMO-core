/// Identity captured from the Core checkout at compile time, never at request time.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildInfo {
    pub git_revision: Option<&'static str>,
    /// An exact tag on the compiled commit; never the nearest tag or latest release.
    pub git_tag: Option<&'static str>,
    /// Tracked modifications at build time; a dirty build is not an exact release.
    pub dirty: Option<bool>,
}

pub fn build_info() -> BuildInfo {
    fn present(value: &'static str) -> Option<&'static str> {
        (!value.is_empty()).then_some(value)
    }
    BuildInfo {
        git_revision: present(env!("MOMO_GIT_REVISION")),
        git_tag: present(env!("MOMO_GIT_TAG")),
        dirty: match env!("MOMO_GIT_DIRTY") {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
    }
}
