use crate::CharacterCard;

impl CharacterCard {
    /// Checks native character definition rules independently of transport and storage.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.chars().count() > 120 {
            return Err("character name must contain between 1 and 120 characters".into());
        }
        semver::Version::parse(&self.version)
            .map_err(|error| format!("character version is not SemVer: {error}"))?;
        if self.author_name.trim().is_empty() || self.author_name.chars().count() > 200 {
            return Err("character author name must contain between 1 and 200 characters".into());
        }
        if let Some(url) = self.author_url.as_deref() {
            url::Url::parse(url)
                .map_err(|error| format!("character author URL is invalid: {error}"))?;
        }
        for (name, markdown) in [
            ("character_markdown", Some(self.character_markdown.as_str())),
            ("user_markdown", Some(self.user_markdown.as_str())),
            ("opening_markdown", self.opening_markdown.as_deref()),
        ] {
            let Some(markdown) = markdown else {
                continue;
            };
            if markdown.len() > 200_000 {
                return Err(format!("{name} exceeds 200000 bytes"));
            }
            let mut lines = markdown.lines();
            if let Some(marker @ ("---" | "+++")) = lines.next().map(str::trim)
                && lines.any(|line| line.trim() == marker)
            {
                return Err(format!("{name} must not contain frontmatter"));
            }
        }
        Ok(())
    }
}
