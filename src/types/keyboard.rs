//! Shared keyboard-layout value types.

/// One XKB layout with an optional variant.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    bincode::Decode,
    bincode::Encode,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct KeyboardLayout {
    pub name: String,
    pub variant: Option<String>,
}

impl KeyboardLayout {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            variant: None,
        }
    }

    pub fn with_variant(name: impl Into<String>, variant: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            variant: Some(variant.into()),
        }
    }
}

impl From<&str> for KeyboardLayout {
    fn from(value: &str) -> Self {
        if let Some((name, variant)) = value
            .strip_suffix(')')
            .and_then(|value| value.rsplit_once('('))
        {
            Self::with_variant(name, variant)
        } else {
            Self::new(value)
        }
    }
}

impl From<String> for KeyboardLayout {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::KeyboardLayout;

    #[test]
    fn parses_xkb_variant_syntax() {
        assert_eq!(
            KeyboardLayout::from("de(nodeadkeys)"),
            KeyboardLayout::with_variant("de", "nodeadkeys")
        );
        assert_eq!(KeyboardLayout::from("us"), KeyboardLayout::new("us"));
    }
}
